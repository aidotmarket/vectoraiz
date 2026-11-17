//! Source-only sale projection v2. Does not call search, rebuild or readback.
//! Private node locations and omission records must never enter the payload.
use super::*;
pub const VERSION: u32 = 2;
const PART_BYTES: u64 = 256_000_000;
const DOCUMENT_BYTES: u64 = 512_000_000;
#[path = "projection_html.rs"]
mod html;
#[path = "projection_pdf.rs"]
mod pdf;
#[path = "projection_graphics.rs"]
mod projected_graphics;
#[cfg(test)]
#[path = "projection_v2_tests.rs"]
mod v2_tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectionNode {
    pub location: String,
    pub kind: String,
    pub text: String,
    pub hidden: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Projection {
    pub version: u32,
    pub nodes: Vec<ProjectionNode>,
    pub drops: BTreeMap<String, usize>,
    pub omissions: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub expanded_bytes: u64,
    pub parts: usize,
}
impl Projection {
    fn new() -> Self {
        Self {
            version: VERSION,
            drops: [
                "deletions",
                "moved-from",
                "comments",
                "instructions",
                "formulas",
                "properties",
                "revision-history",
                "hidden-form-data",
                "unread-parts",
                "headers-footers",
                "metadata",
                "active-markup",
                "attachments",
                "actions",
                "annotations",
                "no-text-pages",
                "extension-data",
            ]
            .map(|k| (k.into(), 0))
            .into(),
            ..Self::default()
        }
    }
    fn drop_at(&mut self, category: &str, location: &str) -> Result<()> {
        self.limit()?;
        *self
            .drops
            .get_mut(category)
            .context("unknown drop category")? += 1;
        self.omissions.push((location.into(), category.into()));
        Ok(())
    }
    fn limit(&self) -> Result<()> {
        ensure!(
            self.nodes.len() + self.omissions.len() < MAX_VALUES,
            "PROJECTION_INVENTORY_LIMIT_EXCEEDED: 5000000"
        );
        Ok(())
    }
    fn add(&mut self, location: &str, kind: &str, text: String, hidden: bool) -> Result<()> {
        self.limit()?;
        if !text.is_empty() {
            self.nodes.push(ProjectionNode {
                location: location.into(),
                kind: kind.into(),
                text,
                hidden,
            });
        }
        Ok(())
    }
}

pub fn extract(kind: &str, input: &[u8]) -> Result<Projection> {
    extract_with_pdfium(kind, input, None)
}

pub fn extract_with_pdfium(
    kind: &str,
    input: &[u8],
    library: Option<&std::path::Path>,
) -> Result<Projection> {
    ensure!(
        input.len() as u64 <= DOCUMENT_BYTES,
        "PROJECTION_INPUT_BYTES_EXCEEDED: {} / {DOCUMENT_BYTES}",
        input.len()
    );
    let mut out = Projection::new();
    if ["html", "htm"].contains(&kind) {
        html::extract(input, &mut out)?;
    } else if kind == "pdf" {
        pdf::extract(input, library, &mut out)?;
    } else if ["txt", "md", "markdown"].contains(&kind) {
        ensure!(
            !input.starts_with(b"PK")
                && !input.starts_with(b"%PDF")
                && !input.starts_with(&[0xd0, 0xcf, 0x11, 0xe0]),
            "container disguised as text"
        );
        let text = std::str::from_utf8(input)?;
        ensure!(!text.contains('\0'), "binary text refused");
        out.expanded_bytes = input.len() as u64;
        out.parts = 1;
        out.add("text/1", "text", text.into(), false)?;
    } else {
        ensure!(
            ["docx", "xlsx", "pptx"].contains(&kind),
            "PROJECTION_FORMAT_UNSUPPORTED: {kind}; exclude this document"
        );
        // Reuse only bounded container/XML decoding, never the originals serializer.
        // Preflight tighter ingestion limits before allocating any expanded part.
        let mut zip = zip::ZipArchive::new(Cursor::new(input))?;
        let mut declared = 0u64;
        for i in 0..zip.len() {
            let p = zip.by_index(i)?;
            declared = declared
                .checked_add(p.size())
                .context("projection size overflow")?;
            ensure!(
                declared <= DOCUMENT_BYTES,
                "PROJECTION_DOCUMENT_BYTES_EXCEEDED: {declared} / {DOCUMENT_BYTES}"
            );
            ensure!(
                p.size() <= PART_BYTES,
                "PROJECTION_PART_BYTES_EXCEEDED at {}: {} / {PART_BYTES}",
                p.name(),
                p.size()
            );
        }
        let (parts, expanded) = read_package(input)?;
        ensure!(
            expanded <= DOCUMENT_BYTES,
            "PROJECTION_DOCUMENT_BYTES_EXCEEDED: {expanded} / {DOCUMENT_BYTES}"
        );
        out.expanded_bytes = expanded;
        out.parts = parts.len();
        let main = match kind {
            "docx" => "word/document.xml",
            "xlsx" => "xl/workbook.xml",
            _ => "ppt/presentation.xml",
        };
        let trees = projection_parts(&parts, main, &mut out)?;
        validate_links(&trees, &parts)?;
        let root_rels = trees
            .get("_rels/.rels")
            .context("missing package root relationships")?;
        let roots: Vec<_> = root_rels
            .children
            .iter()
            .filter(|r| {
                r.attr("Type")
                    .is_some_and(|t| t.ends_with("/officeDocument"))
            })
            .collect();
        ensure!(
            roots.len() == 1
                && resolve("", roots[0].attr("Target").context("package root target")?)? == main,
            "mismatched projection package root"
        );
        for part in parts.keys() {
            if !trees.contains_key(part) {
                out.drop_at("unread-parts", part)?;
            }
            if part.starts_with("docProps/") {
                out.drop_at("properties", part)?;
            }
            if part.to_ascii_lowercase().contains("comment")
                || (kind == "xlsx" && part.ends_with(".vml") && trees.contains_key(part))
            {
                out.drop_at("comments", part)?;
            }
            if part.contains("revision") || part.contains("person") {
                out.drop_at("revision-history", part)?;
            }
        }
        match kind {
            "docx" => {
                ensure!(
                    trees.contains_key("word/document.xml"),
                    "missing DOCX document"
                );
                // Styles can impose inherited hidden text. Until resolved, refuse
                // any style-based vanish rather than assuming visible content.
                if let Some(styles) = trees.get("word/styles.xml") {
                    ensure!(
                        !has(styles, W, "vanish"),
                        "PROJECTION_VISIBILITY_UNKNOWN at word/styles.xml: inherited vanish"
                    );
                }
                for (part, root) in &trees {
                    if part == "word/document.xml"
                        || referenced_word_part(&trees, part)?
                        || ["word/footnotes.xml", "word/endnotes.xml"].contains(&part.as_str())
                    {
                        let mut fields = WordState::default();
                        walk_word(root, part, part, false, &mut fields, &trees, &mut out)?;
                        ensure!(
                            fields.fields.is_empty() && fields.moved.is_empty(),
                            "unclosed field/moved-from range at {part}"
                        );
                    }
                }
            }
            "xlsx" => sheets(&trees, &mut out)?,
            _ => slides(&trees, &mut out)?,
        }
    }
    ensure!(
        out.nodes.iter().any(|n| !n.text.trim().is_empty()),
        "no sale projection text available; exclude document"
    );
    Ok(out)
}
// Decode the projection graph only. Unused XML (including customXml processing
// instructions) is private omission evidence, never a reason to parse its bytes.
fn projection_parts(
    parts: &BTreeMap<String, Vec<u8>>,
    main: &str,
    out: &mut Projection,
) -> Result<BTreeMap<String, Node>> {
    let mut trees = BTreeMap::new();
    let mut pending = vec!["_rels/.rels".to_string(), main.to_string()];
    // A few optional parts have implicit Office relationships. Retain support
    // for minimal producers which omit those relationships.
    for part in [
        "word/styles.xml",
        "word/footnotes.xml",
        "word/endnotes.xml",
        "xl/sharedStrings.xml",
    ] {
        if parts.contains_key(part) && part.split('/').next() == main.split('/').next() {
            pending.push(part.into());
        }
    }
    while let Some(part) = pending.pop() {
        if trees.contains_key(&part) {
            continue;
        }
        let bytes = parts
            .get(&part)
            .with_context(|| format!("missing projection part {part}"))?;
        let root = parse(bytes, &part)?;
        let mut reader = quick_xml::Reader::from_reader(bytes.as_slice());
        let mut number = 0;
        loop {
            match reader.read_event()? {
                Event::Comment(_) => {
                    number += 1;
                    out.drop_at("comments", &format!("{part}/xml-comment[{number}]"))?;
                }
                Event::Eof => break,
                _ => (),
            }
        }
        if part.ends_with(".rels") {
            let own = owner(&part)?;
            for rel in &root.children {
                let typ = rel
                    .attr("Type")
                    .unwrap_or("")
                    .rsplit('/')
                    .next()
                    .unwrap_or("");
                let implicit = [
                    "notesSlide",
                    "slideLayout",
                    "slideMaster",
                    "notesMaster",
                    "styles",
                    "sharedStrings",
                    "footnotes",
                    "endnotes",
                ]
                .contains(&typ);
                let explicit = [
                    "worksheet",
                    "slide",
                    "header",
                    "footer",
                    "table",
                    "drawing",
                    "chart",
                    "diagramData",
                    "diagramDrawing",
                    "vmlDrawing",
                ]
                .contains(&typ);
                let used = trees
                    .get(&own)
                    .is_some_and(|n| uses_relationship(n, rel.attr("Id").unwrap_or("")));
                // Some producers keep the model's cache relationship on the
                // containing slide/document instead of on the data-model part.
                let host_cache = typ == "diagramDrawing"
                    && trees
                        .get(&own)
                        .is_some_and(|n| has(n, graphics::DGM, "relIds"));
                if rel.attr("TargetMode") != Some("External")
                    && (implicit || host_cache || (explicit && used))
                {
                    pending.push(resolve(
                        &own,
                        rel.attr("Target").context("relationship target")?,
                    )?);
                }
            }
        } else if let Some((dir, name)) = part.rsplit_once('/') {
            let rels = format!("{dir}/_rels/{name}.rels");
            if parts.contains_key(&rels) {
                pending.push(rels);
            }
        }
        trees.insert(part, root);
    }
    Ok(trees)
}
fn uses_relationship(n: &Node, id: &str) -> bool {
    n.attrs
        .iter()
        .any(|((ns, key), value)| value == id && (ns == R || key == "relId"))
        || n.children.iter().any(|c| uses_relationship(c, id))
}
fn referenced_word_part(trees: &BTreeMap<String, Node>, part: &str) -> Result<bool> {
    let Some(root) = trees.get("word/document.xml") else {
        return Ok(false);
    };
    let mut sections = Vec::new();
    descendants(root, W, "sectPr", &mut sections);
    for section in sections {
        for reference in &section.children {
            if reference.ns == W
                && ["headerReference", "footerReference"].contains(&reference.local.as_str())
            {
                let id = reference
                    .attr_ns(R, "id")
                    .context("header/footer relationship")?;
                if resolve("word/document.xml", linked(trees, "word/document.xml", id)?)? == part {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
// Paragraphs own edits and warning matching. Visibility changes retain separate
// segments so including hidden text remains an explicit seller decision.
fn paragraph_nodes(out: &mut Projection, start: usize, path: &str, kind: &str) {
    let nodes: Vec<_> = out.nodes.drain(start..).collect();
    let mut segment = 0;
    for mut node in nodes {
        if [
            "text",
            "drawing text",
            "slide text",
            "notes",
            "inline control",
            "chart cached value",
        ]
        .contains(&node.kind.as_str())
        {
            if let Some(last) = out.nodes.last_mut().filter(|last| {
                last.location.starts_with(&format!("{path}/flow["))
                    && last.hidden == node.hidden
                    && last.kind == kind
            }) {
                last.text.push_str(&node.text);
                continue;
            }
            segment += 1;
            node.location = format!("{path}/flow[{segment}]");
            node.kind = kind.into();
        }
        out.nodes.push(node);
    }
}
fn index_entry(instruction: &str) -> bool {
    instruction
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("XE"))
}
fn literal_text(n: &Node) -> bool {
    !n.text.trim().is_empty() || n.children.iter().any(literal_text)
}
// The DPI flag affects rendering only. Do not skip other drawing extensions.
fn local_dpi_extension(n: &Node, path: &str) -> bool {
    n.ns == A
        && n.local == "extLst"
        && path.rsplit_once('/').is_some_and(|(parent, _)| {
            parent
                .rsplit('/')
                .next()
                .is_some_and(|p| p.starts_with("blip["))
        })
        && !literal_text(n)
        && n.children.iter().all(|ext| {
            ext.ns == A
                && ext.local == "ext"
                && ext.children.iter().all(|entry| {
                    entry.ns == "http://schemas.microsoft.com/office/drawing/2010/main"
                        && entry.local == "useLocalDpi"
                        && entry.children.is_empty()
                })
        })
}
fn has(n: &Node, ns: &str, local: &str) -> bool {
    (n.ns == ns && n.local == local) || n.children.iter().any(|c| has(c, ns, local))
}
fn on(value: Option<&str>) -> Result<bool> {
    match value {
        None | Some("1" | "true" | "on") => Ok(true),
        Some("0" | "false" | "off") => Ok(false),
        _ => bail!("PROJECTION_VISIBILITY_UNKNOWN: invalid boolean"),
    }
}
fn validate_links(trees: &BTreeMap<String, Node>, parts: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    for (part, root) in trees.iter().filter(|(p, _)| p.ends_with(".rels")) {
        let own = owner(part)?;
        let mut ids = BTreeSet::new();
        for rel in &root.children {
            ensure!(
                rel.ns == REL && rel.local == "Relationship",
                "unknown relationship at {part}"
            );
            ensure!(
                ids.insert(rel.attr("Id").context("relationship id")?),
                "duplicate relationship id at {part}"
            );
            let target = rel.attr("Target").context("relationship target")?;
            if rel.attr("TargetMode") == Some("External") {
                ensure!(
                    rel.attr("Type").is_some_and(|t| t.ends_with("/hyperlink")),
                    "external content dependency at {part}"
                );
            } else {
                ensure!(
                    rel.attr("TargetMode").is_none() || rel.attr("TargetMode") == Some("Internal"),
                    "unknown relationship mode at {part}"
                );
                ensure!(
                    parts.contains_key(&resolve(&own, target)?),
                    "missing relationship target at {part}"
                );
            }
        }
    }
    Ok(())
}
#[derive(Default)]
struct WordState {
    fields: Vec<(bool, String)>,
    moved: BTreeSet<String>,
}
#[allow(clippy::too_many_arguments)]
fn walk_word(
    n: &Node,
    path: &str,
    host: &str,
    hidden: bool,
    fields: &mut WordState,
    trees: &BTreeMap<String, Node>,
    out: &mut Projection,
) -> Result<()> {
    if (n.ns == graphics::C && n.local == "chart") || (n.ns == graphics::DGM && n.local == "relIds")
    {
        if !fields.moved.is_empty() {
            return out.drop_at("moved-from", path);
        }
        if fields.fields.iter().any(|field| !field.0) {
            return out.drop_at("instructions", path);
        }
    }
    if n.ns == MC || n.ns == "urn:schemas-microsoft-com:vml" {
        bail!("PROJECTION_WORD_TEXTBOX_UNSUPPORTED at {path}: VML/AlternateContent requires a proven branch and visibility");
    }
    if n.ns == "http://schemas.microsoft.com/office/word/2010/wordml" && path.contains("/rPr[") {
        ensure!(
            !literal_text(n),
            "PROJECTION_CONTENT_UNSUPPORTED at {path}: w14 run property text"
        );
        return Ok(());
    }
    if local_dpi_extension(n, path) {
        return Ok(());
    }
    if projected_graphics::reference(n, host, path, hidden, trees, out)? {
        return Ok(());
    }
    ensure!(
        [W, A, WP, PIC, R, XML].contains(&n.ns.as_str()),
        "PROJECTION_NAMESPACE_UNSUPPORTED at {path}: {}",
        n.ns
    );
    ensure!(
        !["altChunk", "object", "subDoc", "contentPart"].contains(&n.local.as_str()),
        "PROJECTION_CONTENT_UNSUPPORTED at {path}"
    );
    let paragraph_start = out.nodes.len();
    if n.ns == W {
        if (n.local == "tr"
            && n.children
                .iter()
                .any(|c| c.ns == W && c.local == "trPr" && has(c, W, "del")))
            || (n.local == "tc"
                && n.children
                    .iter()
                    .any(|c| c.ns == W && c.local == "tcPr" && has(c, W, "cellDel")))
        {
            return out.drop_at("deletions", path);
        }
        if let Some(category) = match n.local.as_str() {
            "del" | "delText" | "delInstrText" => Some("deletions"),
            "moveFrom" => Some("moved-from"),
            "instrText" => {
                if let Some(field) = fields.fields.last_mut() {
                    field.1.push_str(&n.text);
                }
                Some("instructions")
            }
            "comment" | "commentReference" | "commentRangeStart" | "commentRangeEnd" => {
                Some("comments")
            }
            _ => None,
        } {
            return out.drop_at(category, path);
        }
        for ((_, key), _) in n
            .attrs
            .iter()
            .filter(|((_, k), _)| words("author date rsidR rsidDel rsidRPr", k))
        {
            out.drop_at("revision-history", &format!("{path}/@{key}"))?;
        }
        if n.local == "moveFromRangeStart" {
            ensure!(
                fields
                    .moved
                    .insert(n.attr_ns(W, "id").context("moved-from range id")?.into()),
                "duplicate moved-from range at {path}"
            );
            return out.drop_at("moved-from", path);
        }
        if n.local == "moveFromRangeEnd" {
            ensure!(
                fields
                    .moved
                    .remove(n.attr_ns(W, "id").context("moved-from range id")?),
                "moved-from range end without start at {path}"
            );
            return Ok(());
        }
        if n.local == "fldChar" {
            match n.attr_ns(W, "fldCharType") {
                Some("begin") => {
                    fields.fields.push((false, String::new()));
                    out.drop_at("instructions", path)?;
                }
                Some("separate") => {
                    let f = fields
                        .fields
                        .last_mut()
                        .context("field separator without begin")?;
                    ensure!(!f.0, "duplicate field separator at {path}");
                    f.0 = true;
                }
                Some("end") => {
                    let field = fields.fields.pop().context("field end without begin")?;
                    ensure!(
                        field.0 || index_entry(&field.1),
                        "PROJECTION_FIELD_RESULT_UNPROVEN at {path}: field has no displayed result"
                    );
                }
                _ => bail!("unknown field state at {path}"),
            }
            return Ok(());
        }
        if n.local == "fldSimple" {
            ensure!(
                has(n, W, "t") || index_entry(n.attr_ns(W, "instr").unwrap_or("")),
                "PROJECTION_FIELD_RESULT_UNPROVEN at {path}: field has no displayed result"
            );
            out.drop_at("instructions", &format!("{path}/@instr"))?;
        }
    }
    let hidden = hidden
        || own_hidden_shape(n)?
        || (n.ns == WP
            && n.local == "docPr"
            && n.attr("hidden")
                .map(|v| on(Some(v)))
                .transpose()?
                .unwrap_or(false))
        || (n.ns == W
            && n.local == "tr"
            && n.children
                .iter()
                .filter(|c| c.ns == W && c.local == "trPr")
                .flat_map(|c| &c.children)
                .any(|p| {
                    p.ns == W
                        && p.local == "hidden"
                        && p.attr_ns(W, "val")
                            .is_none_or(|v| !["0", "false", "off"].contains(&v))
                }))
        || (n.ns == W && n.local == "drawing" && {
            let mut props = Vec::new();
            descendants(n, WP, "docPr", &mut props);
            props.iter().any(|p| {
                p.attr("hidden")
                    .is_some_and(|v| ["1", "true", "on"].contains(&v))
            })
        })
        || (n.ns == W
            && n.local == "r"
            && n.children
                .iter()
                .filter(|c| c.ns == W && c.local == "rPr")
                .flat_map(|p| &p.children)
                .any(|p| {
                    p.ns == W
                        && p.local == "vanish"
                        && p.attr_ns(W, "val")
                            .is_none_or(|v| !["0", "false", "off"].contains(&v))
                }));
    if n.ns == W && ["vanish", "hidden"].contains(&n.local.as_str()) {
        on(n.attr_ns(W, "val"))?;
    }
    if n.ns == W && n.local == "t" {
        if !fields.moved.is_empty() {
            out.drop_at("moved-from", path)?;
        } else if fields.fields.iter().any(|field| !field.0) {
            out.drop_at("instructions", path)?;
        } else {
            out.add(path, "text", n.text.clone(), hidden)?;
        }
    }
    if n.ns == A && n.local == "t" {
        ensure!(
            fields.fields.is_empty() && fields.moved.is_empty(),
            "drawing text inside complex field at {path}"
        );
        out.add(path, "drawing text", n.text.clone(), hidden)?;
    }
    if ((n.ns == W && ["br", "cr"].contains(&n.local.as_str())) || (n.ns == A && n.local == "br"))
        && fields.moved.is_empty()
        && !fields.fields.iter().any(|field| !field.0)
    {
        out.add(path, "inline control", "\n".into(), hidden)?;
    }
    if (n.local == "docPr" && n.ns == WP) || (n.local == "cNvPr" && n.ns == PIC) {
        for key in ["descr", "title"] {
            if let Some(value) = n.attr(key) {
                let location = format!("{path}/@{key}");
                if !fields.moved.is_empty() {
                    out.drop_at("moved-from", &location)?;
                } else if fields.fields.iter().any(|field| !field.0) {
                    out.drop_at("instructions", &location)?;
                } else {
                    out.add(&location, "alt text", value.into(), hidden)?;
                }
            }
        }
    }
    for (i, c) in n.children.iter().enumerate() {
        if n.ns == W
            && n.local == "r"
            && c.ns == W
            && c.local == "tab"
            && fields.moved.is_empty()
            && !fields.fields.iter().any(|field| !field.0)
        {
            out.add(
                &format!("{path}/tab[{}]", i + 1),
                "inline control",
                "\t".into(),
                hidden,
            )?;
        }
        walk_word(
            c,
            &format!("{path}/{}[{}]", c.local, i + 1),
            host,
            hidden,
            fields,
            trees,
            out,
        )?;
    }
    if [W, A].contains(&n.ns.as_str()) && n.local == "p" {
        paragraph_nodes(out, paragraph_start, path, "paragraph");
    }
    Ok(())
}
fn linked<'a>(trees: &'a BTreeMap<String, Node>, part: &str, id: &str) -> Result<&'a str> {
    let (dir, name) = part.rsplit_once('/').context("part directory")?;
    let rels = trees
        .get(&format!("{dir}/_rels/{name}.rels"))
        .context("missing relationships")?;
    let rel = rels
        .children
        .iter()
        .find(|n| n.attr("Id") == Some(id))
        .context("unresolved relationship")?;
    rel.attr("Target").context("missing target")
}
fn descendants<'a>(n: &'a Node, ns: &str, local: &str, out: &mut Vec<&'a Node>) {
    if n.ns == ns && n.local == local {
        out.push(n);
    }
    for c in &n.children {
        descendants(c, ns, local, out);
    }
}
fn sheets(trees: &BTreeMap<String, Node>, out: &mut Projection) -> Result<()> {
    let workbook = trees.get("xl/workbook.xml").context("missing workbook")?;
    let mut sheet_nodes = Vec::new();
    descendants(workbook, S, "sheet", &mut sheet_nodes);
    let mut shared = Vec::new();
    if let Some(root) = trees.get("xl/sharedStrings.xml") {
        descendants(root, S, "si", &mut shared);
    }
    for sheet in sheet_nodes {
        let hidden = match sheet.attr("state") {
            None | Some("visible") => false,
            Some("hidden" | "veryHidden") => true,
            _ => bail!("PROJECTION_VISIBILITY_UNKNOWN at xl/workbook.xml"),
        };
        let target = resolve(
            "xl/workbook.xml",
            linked(
                trees,
                "xl/workbook.xml",
                sheet.attr_ns(R, "id").context("sheet relationship")?,
            )?,
        )?;
        let root = trees.get(&target).context("missing worksheet")?;
        ensure!(
            root.ns == S && root.local == "worksheet",
            "unsupported sheet type at {target}"
        );
        ensure!(
            !["oleObject", "control", "legacyDrawingHF"]
                .iter()
                .any(|local| has(root, S, local)),
            "PROJECTION_CONTENT_UNSUPPORTED at {target}: worksheet carrier/extension"
        );
        let mut legacy = Vec::new();
        descendants(root, S, "legacyDrawing", &mut legacy);
        for (i, reference) in legacy.iter().enumerate() {
            let part = resolve(
                &target,
                linked(
                    trees,
                    &target,
                    reference
                        .attr_ns(R, "id")
                        .context("legacy drawing relationship")?,
                )?,
            )?;
            let carrier = trees.get(&part).context("missing legacy drawing")?;
            let mut shapes = Vec::new();
            descendants(
                carrier,
                "urn:schemas-microsoft-com:vml",
                "shape",
                &mut shapes,
            );
            ensure!(
                !shapes.is_empty()
                    && shapes.iter().all(|shape| {
                        let mut data = Vec::new();
                        descendants(
                            shape,
                            "urn:schemas-microsoft-com:office:excel",
                            "ClientData",
                            &mut data,
                        );
                        data.len() == 1 && data[0].attr("ObjectType") == Some("Note")
                    }),
                "PROJECTION_CONTENT_UNSUPPORTED at {part}: non-comment VML carrier"
            );
            out.drop_at("comments", &format!("{target}/legacyDrawing[{i}]"))?;
        }
        let mut headers = Vec::new();
        descendants(root, S, "headerFooter", &mut headers);
        for (i, header) in headers.iter().enumerate() {
            if !header.children.is_empty() {
                out.drop_at("headers-footers", &format!("{target}/headerFooter[{i}]"))?;
            }
        }
        let mut extensions = Vec::new();
        descendants(root, S, "extLst", &mut extensions);
        let mut rules = Vec::new();
        descendants(root, S, "cfRule", &mut rules);
        let data_bar_extensions: std::collections::HashSet<*const Node> = rules
            .iter()
            .filter(|rule| rule.attr("type") == Some("dataBar"))
            .flat_map(|rule| {
                rule.children
                    .iter()
                    .filter(|child| child.ns == S && child.local == "extLst")
                    .map(|child| child as *const Node)
            })
            .collect();
        for extension in extensions {
            ensure!(
                extension
                    .children
                    .iter()
                    .all(|ext| ext.children.iter().all(|entry| entry.ns
                        == "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"
                        && (["conditionalFormattings", "sparklineGroups"]
                            .contains(&entry.local.as_str())
                            || (entry.local == "id"
                                && entry.children.is_empty()
                                && data_bar_extensions.contains(&(extension as *const Node))))
                        && !has(entry, S, "t")
                        && !has(entry, A, "t"))),
                "PROJECTION_CONTENT_UNSUPPORTED at {target}: content-bearing worksheet extension"
            );
            for entry in extension
                .children
                .iter()
                .flat_map(|ext| &ext.children)
                .filter(|entry| entry.local == "id" && !entry.text.is_empty())
            {
                let _ = entry;
                out.drop_at(
                    "extension-data",
                    &format!("{target}/x14:id[{}]", out.drops["extension-data"] + 1),
                )?;
            }
        }
        let mut formats = Vec::new();
        descendants(root, S, "sheetFormatPr", &mut formats);
        ensure!(
            !formats.iter().any(|f| f
                .attr("zeroHeight")
                .is_some_and(|v| !["0", "false", "off"].contains(&v))),
            "PROJECTION_VISIBILITY_UNKNOWN at {target}: default hidden rows"
        );
        out.add(
            &format!("{target}/@name"),
            "sheet name",
            sheet.attr("name").context("sheet name")?.into(),
            hidden,
        )?;
        let mut cols = Vec::new();
        descendants(root, S, "col", &mut cols);
        let mut hidden_cols = Vec::new();
        for c in cols {
            if c.attr("hidden")
                .map(|v| on(Some(v)))
                .transpose()?
                .unwrap_or(false)
                || c.attr("width").is_some_and(|v| v.parse::<f64>() == Ok(0.0))
            {
                hidden_cols.push((
                    c.attr("min").context("column min")?.parse::<i64>()?,
                    c.attr("max").context("column max")?.parse::<i64>()?,
                ));
            }
        }
        let rows: Vec<_> = root
            .children
            .iter()
            .filter(|n| n.ns == S && n.local == "sheetData")
            .flat_map(|n| &n.children)
            .filter(|n| n.ns == S && n.local == "row")
            .collect();
        let mut hidden_rows = BTreeSet::new();
        for row in rows {
            let row_hidden = hidden
                || row
                    .attr("hidden")
                    .map(|v| on(Some(v)))
                    .transpose()?
                    .unwrap_or(false)
                || row.attr("ht").is_some_and(|v| v.parse::<f64>() == Ok(0.0));
            if row_hidden {
                hidden_rows.insert(row.attr("r").context("row coordinate")?.parse::<i64>()?);
            }
            for cell in row.children.iter().filter(|n| n.ns == S && n.local == "c") {
                let reference = cell.attr("r").context("cell coordinate")?;
                let (col, _) = cell_ref(reference)?;
                let cell_hidden =
                    row_hidden || hidden_cols.iter().any(|(lo, hi)| col >= *lo && col <= *hi);
                let loc = format!("{target}/cell/{reference}");
                let formula = cell.children.iter().any(|n| n.ns == S && n.local == "f");
                if formula {
                    out.drop_at("formulas", &loc)?;
                }
                let value = cell.children.iter().find(|n| n.ns == S && n.local == "v");
                ensure!(!formula || value.is_some(), "PROJECTION_MISSING_CACHE at {loc}: recalculate/save locally or exclude document");
                let text = match cell.attr("t") {
                    Some("s") => {
                        let i: usize =
                            value.context("missing shared-string index")?.text.parse()?;
                        string_text(shared.get(i).context("shared-string index out of bounds")?)?
                    }
                    Some("inlineStr") => string_text(cell)?,
                    Some("e") => {
                        out.warnings.push(format!("{loc}: cached error retained"));
                        value.context("missing cached error")?.text.clone()
                    }
                    None | Some("n" | "b" | "str" | "d") => {
                        value.map(|n| n.text.clone()).unwrap_or_default()
                    }
                    _ => bail!("unsupported cached cell type at {loc}"),
                };
                out.add(&loc, "cached cell value", text, cell_hidden)?;
            }
        }
        let mut table_refs = Vec::new();
        descendants(root, S, "tablePart", &mut table_refs);
        for table_ref in table_refs {
            let part = resolve(
                &target,
                linked(
                    trees,
                    &target,
                    table_ref.attr_ns(R, "id").context("table relationship")?,
                )?,
            )?;
            let table = trees.get(&part).context("missing table")?;
            out.add(
                &format!("{part}/@name"),
                "table name",
                table
                    .attr("displayName")
                    .or(table.attr("name"))
                    .context("table name")?
                    .into(),
                hidden,
            )?;
            let start = table
                .attr("ref")
                .context("table reference")?
                .split(':')
                .next()
                .context("table start")?;
            let (first_col, first_row) = cell_ref(start)?;
            let mut headers = Vec::new();
            descendants(table, S, "tableColumn", &mut headers);
            for (index, header) in headers.iter().enumerate() {
                let col = first_col + index as i64;
                let hidden = hidden
                    || hidden_rows.contains(&first_row)
                    || hidden_cols.iter().any(|(lo, hi)| col >= *lo && col <= *hi);
                out.add(
                    &format!("{part}/tableColumn[{}]/@name", index + 1),
                    "table header",
                    header.attr("name").context("table header name")?.into(),
                    hidden,
                )?;
            }
        }
        let mut drawings = Vec::new();
        descendants(root, S, "drawing", &mut drawings);
        for drawing in drawings {
            ensure!(hidden || (hidden_rows.is_empty() && hidden_cols.is_empty()),"PROJECTION_VISIBILITY_UNKNOWN at {target}: drawing anchored across hidden rows/columns");
            let part = resolve(
                &target,
                linked(
                    trees,
                    &target,
                    drawing.attr_ns(R, "id").context("drawing relationship")?,
                )?,
            )?;
            walk_slide(
                trees.get(&part).context("missing worksheet drawing")?,
                &part,
                &part,
                hidden,
                trees,
                out,
            )?;
        }
    }
    // Count formula-bearing metadata without ever extracting it as content.
    for (part, root) in trees {
        for ns in [X14, XM] {
            for local in ["f", "formula", "formula1", "formula2"] {
                let mut nodes = Vec::new();
                descendants(root, ns, local, &mut nodes);
                for (i, _) in nodes.iter().enumerate() {
                    out.drop_at("formulas", &format!("{part}/{{{ns}}}{local}[{i}]"))?;
                }
            }
        }
        for local in ["definedName", "formula", "formula1", "formula2"] {
            let mut nodes = Vec::new();
            descendants(root, S, local, &mut nodes);
            for (i, _) in nodes.iter().enumerate() {
                out.drop_at("formulas", &format!("{part}/{local}[{i}]"))?;
            }
        }
    }
    Ok(())
}
fn string_text(n: &Node) -> Result<String> {
    ensure!(
        !has(n, S, "rPh"),
        "PROJECTION_VISIBILITY_UNKNOWN: phonetic text"
    );
    let mut nodes = Vec::new();
    descendants(n, S, "t", &mut nodes);
    Ok(nodes
        .iter()
        .map(|n| n.text.as_str())
        .collect::<Vec<_>>()
        .join(""))
}
fn slides(trees: &BTreeMap<String, Node>, out: &mut Projection) -> Result<()> {
    let presentation = trees
        .get("ppt/presentation.xml")
        .context("missing presentation")?;
    let mut slides = Vec::new();
    descendants(presentation, P, "sldId", &mut slides);
    for slide in slides {
        let target = resolve(
            "ppt/presentation.xml",
            linked(
                trees,
                "ppt/presentation.xml",
                slide.attr_ns(R, "id").context("slide relationship")?,
            )?,
        )?;
        let root = trees.get(&target).context("missing slide")?;
        let hidden = !root
            .attr("show")
            .map(|v| on(Some(v)))
            .transpose()?
            .unwrap_or(true);
        walk_slide(root, &target, &target, hidden, trees, out)?;
        let (dir, name) = target.rsplit_once('/').context("slide directory")?;
        if let Some(rels) = trees.get(&format!("{dir}/_rels/{name}.rels")) {
            for rel in &rels.children {
                if rel.attr("Type").is_some_and(|s| s.ends_with("/notesSlide")) {
                    let notes = resolve(&target, rel.attr("Target").context("notes target")?)?;
                    walk_slide(
                        trees.get(&notes).context("missing notes")?,
                        &notes,
                        &notes,
                        hidden,
                        trees,
                        out,
                    )?;
                }
                if rel
                    .attr("Type")
                    .is_some_and(|s| s.ends_with("/slideLayout"))
                {
                    let layout = resolve(&target, rel.attr("Target").context("layout target")?)?;
                    refuse_inherited_text(trees, &layout, 0)?;
                }
            }
        }
    }
    Ok(())
}
fn walk_slide(
    n: &Node,
    path: &str,
    host: &str,
    hidden: bool,
    trees: &BTreeMap<String, Node>,
    out: &mut Projection,
) -> Result<()> {
    if projected_graphics::reference(n, host, path, hidden, trees, out)? {
        return Ok(());
    }
    // PowerPoint writes creation IDs, hyperlink colours and media metadata here.
    // Any literal text still refuses rather than disappearing from projection.
    if n.local == "extLst" && [P, A].contains(&n.ns.as_str()) {
        let mut charts = Vec::new();
        descendants(n, graphics::C, "chart", &mut charts);
        ensure!(charts.is_empty(), "PROJECTION_CONTENT_UNSUPPORTED at {path}: {} chart references only in slide/drawing extLst", charts.len());
        ensure!(
            !literal_text(n),
            "PROJECTION_CONTENT_UNSUPPORTED at {path}: text-bearing drawing/slide extension"
        );
        return Ok(());
    }
    let paragraph_start = out.nodes.len();
    ensure!(
        [P, A, PIC, XDR, R, WP].contains(&n.ns.as_str()),
        "PROJECTION_NAMESPACE_UNSUPPORTED at {path}: {}",
        n.ns
    );
    ensure!(
        n.ns != MC && n.local != "oleObj",
        "PROJECTION_CONTENT_UNSUPPORTED at {path}"
    );
    let hidden = hidden || own_hidden_shape(n)?;
    if n.local == "cNvPr" {
        let hidden = hidden
            || n.attr("hidden")
                .map(|v| on(Some(v)))
                .transpose()?
                .unwrap_or(false);
        for key in ["descr", "title"] {
            if let Some(s) = n.attr(key) {
                out.add(&format!("{path}/@{key}"), "alt text", s.into(), hidden)?;
            }
        }
    }
    if n.ns == A && n.local == "t" {
        out.add(
            path,
            if path.contains("notes") {
                "notes"
            } else {
                "slide text"
            },
            n.text.clone(),
            hidden,
        )?;
    }
    if n.ns == A && n.local == "br" {
        out.add(path, "inline control", "\n".into(), hidden)?;
    }
    if n.ns == A && n.local == "fld" {
        out.drop_at("instructions", &format!("{path}/@type"))?;
    }
    for (i, c) in n.children.iter().enumerate() {
        walk_slide(
            c,
            &format!("{path}/{}[{}]", c.local, i + 1),
            host,
            hidden,
            trees,
            out,
        )?;
    }
    if n.ns == A && n.local == "p" {
        paragraph_nodes(
            out,
            paragraph_start,
            path,
            if path.contains("notes") {
                "notes paragraph"
            } else {
                "slide paragraph"
            },
        );
    }
    Ok(())
}
fn own_hidden_shape(n: &Node) -> Result<bool> {
    for props in n.children.iter().filter(|c| {
        [
            "nvSpPr",
            "nvPicPr",
            "nvGrpSpPr",
            "nvGraphicFramePr",
            "nvCxnSpPr",
        ]
        .contains(&c.local.as_str())
    }) {
        for c in &props.children {
            if c.local == "cNvPr"
                && c.attr("hidden")
                    .map(|v| on(Some(v)))
                    .transpose()?
                    .unwrap_or(false)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn refuse_inherited_text(trees: &BTreeMap<String, Node>, part: &str, depth: usize) -> Result<()> {
    ensure!(
        depth <= 8,
        "PROJECTION_REFERENCE_DEPTH_EXCEEDED at {part}: 8"
    );
    let root = trees.get(part).context("missing inherited part")?;
    ensure!(
        !inherited_content(root),
        "PROJECTION_LAYOUT_UNSUPPORTED at {part}: inherited content requires a proven projection"
    );
    let (dir, name) = part.rsplit_once('/').context("inherited part path")?;
    if let Some(rels) = trees.get(&format!("{dir}/_rels/{name}.rels")) {
        for rel in &rels.children {
            if rel
                .attr("Type")
                .is_some_and(|t| t.ends_with("/slideMaster") || t.ends_with("/notesMaster"))
            {
                refuse_inherited_text(
                    trees,
                    &resolve(part, rel.attr("Target").context("inherited target")?)?,
                    depth + 1,
                )?;
            }
        }
    }
    Ok(())
}

fn inherited_content(n: &Node) -> bool {
    // Layout/master placeholder prompts are authoring UI, not rendered content.
    if n.ns == P && ["sp", "graphicFrame", "pic"].contains(&n.local.as_str()) && has(n, P, "ph") {
        return false;
    }
    (n.local == "cNvPr"
        && ["descr", "title"]
            .iter()
            .any(|key| n.attr(key).is_some_and(|v| !v.trim().is_empty())))
        || (n.ns == A && n.local == "t")
        || (n.ns == graphics::C && n.local == "chart")
        || (n.ns == graphics::DGM && n.local == "relIds")
        || n.children.iter().any(inherited_content)
}
