//! Tolerant, page-scoped extraction. Never invoke the listing sanitizer during ingest.
use super::*;
use std::{fs::File, io::Read};

fn read_bytes(zip: &mut zip::ZipArchive<File>, part: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    zip.by_name(part)?
        .take(crate::parsers::MAX_PART)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn read(zip: &mut zip::ZipArchive<File>, part: &str) -> Result<Node> {
    parse(&read_bytes(zip, part)?, part)
}
fn relationships(zip: &mut zip::ZipArchive<File>, part: &str) -> Result<Vec<Node>> {
    Ok(read(zip, &graphics::relpath(part)?)?.children)
}
pub(super) fn sheet_parts(zip: &mut zip::ZipArchive<File>) -> Result<BTreeMap<String, String>> {
    let workbook = read(zip, "xl/workbook.xml")?;
    let rels = relationships(zip, "xl/workbook.xml")?;
    let mut parts = BTreeMap::new();
    for sheet in workbook
        .children
        .iter()
        .filter(|n| n.ns == S && n.local == "sheets")
        .flat_map(|n| &n.children)
    {
        if let (Some(name), Some(id)) = (sheet.attr("name"), sheet.attr_ns(R, "id")) {
            if let Some(rel) = rels
                .iter()
                .find(|r| r.attr("Id") == Some(id) && r.attr("TargetMode") != Some("External"))
            {
                if let Some(target) = rel
                    .attr("Target")
                    .and_then(|t| resolve("xl/workbook.xml", t).ok())
                {
                    parts.insert(name.to_owned(), target);
                }
            }
        }
    }
    Ok(parts)
}
// Host worksheets can be large. Stream their references instead of building a
// DOM or invoking any listing inspection/rewrite pass.
fn references(bytes: &[u8]) -> Result<(String, BTreeSet<String>)> {
    let mut reader = NsReader::from_reader(bytes);
    let mut ids = BTreeSet::new();
    let mut root_ns = String::new();
    let mut first = true;
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let (ns, local) = reader.resolve_element(e.name());
                if first {
                    if let ResolveResult::Bound(ns) = ns {
                        root_ns = String::from_utf8(ns.as_ref().to_vec())?;
                    }
                    first = false;
                }
                for attr in e.attributes() {
                    let attr = attr?;
                    let (ns, key) = reader.resolve_attribute(attr.key);
                    if (matches!(ns, ResolveResult::Bound(n) if n.as_ref() == R.as_bytes())
                        && words("id dm lo qs cs", std::str::from_utf8(key.as_ref())?))
                        || (local.as_ref() == b"dataModelExt" && attr.key.as_ref() == b"relId")
                    {
                        ids.insert(
                            attr.decode_and_unescape_value(reader.decoder())?
                                .into_owned(),
                        );
                    }
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }
    Ok((root_ns, ids))
}
fn display_text(n: &Node, out: &mut String) {
    // Review inventory presents DrawingML runs joined within each paragraph.
    if n.ns == A && n.local == "p" {
        out.push_str(&graphics::text(n));
        out.push('\n');
        return;
    }
    if (n.ns == A && n.local == "t") || (n.ns == graphics::C && words("v separator", &n.local)) {
        out.push_str(&n.text);
        out.push('\n');
    }
    for child in &n.children {
        display_text(child, out);
    }
}
pub(super) fn graphics_text(zip: &mut zip::ZipArchive<File>, host: &str) -> String {
    let mut pending = vec![(host.to_owned(), None)];
    let mut visited = BTreeSet::new();
    let mut cache_models = BTreeMap::new();
    let mut text = String::new();
    while let Some((part, parent)) = pending.pop() {
        if !visited.insert(part.clone()) {
            continue;
        }
        let Ok(bytes) = read_bytes(zip, &part) else {
            continue;
        };
        let Ok((ns, ids)) = references(&bytes) else {
            continue;
        };
        if ns == graphics::DGM {
            let Ok(root) = parse(&bytes, &part) else {
                continue;
            };
            if root.local != "dataModel" {
                continue;
            }
            // Prefer the resolved drawing cache. A missing, external, ambiguous
            // or unreadable cache falls back to the model during tolerant ingest.
            let cache = (|| -> Option<(String, Node)> {
                // dataModelExt may refer to the host or to the data part itself.
                let mut extensions = Vec::new();
                graphics::visit(&root, graphics::DSP, "dataModelExt", &mut extensions);
                if extensions.len() != 1 {
                    return None;
                }
                let id = extensions[0].attr("relId")?;
                let mut targets = BTreeSet::new();
                for owner in std::iter::once(&part).chain(parent.as_ref()) {
                    for rel in relationships(zip, owner).unwrap_or_default() {
                        if rel.attr("Id") == Some(id)
                            && rel.attr("TargetMode") != Some("External")
                            && rel.attr("Type") == Some("http://schemas.microsoft.com/office/2007/relationships/diagramDrawing")
                        {
                            if let Some(target) = rel.attr("Target").and_then(|t| resolve(owner, t).ok()) {
                                targets.insert(target);
                            }
                        }
                    }
                }
                if targets.len() != 1 {
                    return None;
                }
                let target = targets.pop_first()?;
                let mut drawing = read(zip, &target).ok()?;
                if drawing.ns != graphics::DSP || drawing.local != "drawing" {
                    return None;
                }
                graphics::resolve_alternates(&mut drawing, &target, &mut Report::default()).ok()?;
                Some((target, drawing))
            })();
            if let Some((target, drawing)) = cache {
                let previous_model = cache_models.insert(target.clone(), part.clone());
                if visited.insert(target) {
                    display_text(&drawing, &mut text);
                } else if previous_model.is_some_and(|owner| owner != part) {
                    // A different model consumed this shared cache. A cache
                    // reached directly still counts as this model's display.
                    display_text(&root, &mut text);
                }
            } else {
                display_text(&root, &mut text);
            }
            continue;
        }
        if ns == graphics::C || ns == graphics::DSP {
            let Ok(mut root) = parse(&bytes, &part) else {
                continue;
            };
            let mut report = Report::default();
            if graphics::resolve_alternates(&mut root, &part, &mut report).is_err() {
                continue;
            }
            if ns != graphics::C || graphics::chart(&mut root, &part, &mut report, false).is_ok() {
                display_text(&root, &mut text);
            }
            continue; // Never follow externalData into a workbook.
        }
        let Ok(rels) = relationships(zip, &part) else {
            continue;
        };
        for rel in rels.into_iter().rev() {
            if rel.attr("TargetMode") == Some("External")
                || !rel.attr("Id").is_some_and(|id| ids.contains(id))
            {
                continue;
            }
            let kind = rel
                .attr("Type")
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("");
            // Definitions carry no display text. Only a host's drawing, chart,
            // or SmartArt data model/cache participates in its page.
            if !words("drawing chart diagramData diagramDrawing", kind) {
                continue;
            }
            if let Some(target) = rel.attr("Target").and_then(|t| resolve(&part, t).ok()) {
                pending.push((target, Some(part.clone())));
            }
        }
    }
    text
}
