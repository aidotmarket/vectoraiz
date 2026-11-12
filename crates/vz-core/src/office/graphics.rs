//! Static graphics preparation. Dependency deletion is scoped to removed references.
use super::*;
pub(super) const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
pub(super) const CS: &str = "http://schemas.microsoft.com/office/drawing/2012/chartStyle";
pub(super) const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
pub(super) const DSP: &str = "http://schemas.microsoft.com/office/drawing/2008/diagram";
fn visit<'a>(n: &'a Node, ns: &str, local: &str, out: &mut Vec<&'a Node>) {
    if n.ns == ns && n.local == local {
        out.push(n);
    }
    for c in &n.children {
        visit(c, ns, local, out);
    }
}
fn text(n: &Node) -> String {
    let mut nodes = Vec::new();
    visit(n, A, "t", &mut nodes);
    nodes
        .iter()
        .map(|n| n.text.as_str())
        .collect::<Vec<_>>()
        .join("")
}
fn relpath(part: &str) -> Result<String> {
    let (dir, file) = part.rsplit_once('/').context("graphics owner path")?;
    Ok(format!("{dir}/_rels/{file}.rels"))
}
/// dataModelExt is found in the data model but commonly names a relationship
/// belonging to its host slide; support either owner only when unambiguous.
fn drawing_for(
    data: &str,
    root: &Node,
    trees: &BTreeMap<String, Node>,
    rels: &BTreeMap<String, Node>,
) -> Result<String> {
    let mut exts = Vec::new();
    visit(root, DSP, "dataModelExt", &mut exts);
    ensure!(
        exts.len() == 1,
        "SmartArt drawing cache missing; layout rendering unsupported at {data}"
    );
    let id = exts[0]
        .attr("relId")
        .context("SmartArt drawing relationship missing")?;
    let mut owners = BTreeSet::from([data.to_owned()]);
    for (rp, n) in rels {
        let own = owner(rp)?;
        for r in &n.children {
            if r.attr("TargetMode") != Some("External")
                && r.attr("Type").is_some_and(|t| t.ends_with("/diagramData"))
                && resolve(&own, r.attr("Target").context("diagram target missing")?)? == data
            {
                owners.insert(own.clone());
            }
        }
    }
    let mut targets = BTreeSet::new();
    for own in owners {
        if let Some(rs) = rels.get(&relpath(&own)?) {
            for r in &rs.children {
                if r.attr("Id") == Some(id)
                    && r.attr("TargetMode") != Some("External")
                    && r.attr("Type")
                        == Some(
                            "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing",
                        )
                {
                    targets.insert(resolve(
                        &own,
                        r.attr("Target").context("diagram target absent")?,
                    )?);
                }
            }
        }
    }
    ensure!(
        targets.len() == 1,
        "SmartArt drawing cache missing; layout rendering unsupported at {data}"
    );
    let target = targets.into_iter().next().unwrap();
    ensure!(
        trees
            .get(&target)
            .is_some_and(|n| n.ns == DSP && n.local == "drawing"),
        "SmartArt drawing cache missing; layout rendering unsupported at {data}"
    );
    Ok(target)
}
fn diagram_models(
    trees: &mut BTreeMap<String, Node>,
    rels: &BTreeMap<String, Node>,
    report: &mut Report,
) -> Result<()> {
    let data: Vec<_> = trees
        .iter()
        .filter(|(_, n)| n.ns == DGM && n.local == "dataModel")
        .map(|(p, _)| p.clone())
        .collect();
    for part in data {
        let root = &trees[&part];
        let target = drawing_for(&part, root, trees, rels)?;
        let mut shapes = Vec::new();
        visit(&trees[&target], DSP, "sp", &mut shapes);
        ensure!(
            !shapes.is_empty(),
            "SmartArt drawing cache missing; layout rendering unsupported at {part}"
        );
        let mut cache = BTreeMap::new();
        for shape in shapes {
            let id = shape
                .attr("modelId")
                .context("SmartArt shape lacks modelId")?;
            ensure!(
                cache.insert(id.to_owned(), text(shape)).is_none(),
                "ambiguous SmartArt drawing shape at {target}/{id}"
            );
        }
        let mut connections = Vec::new();
        visit(root, DGM, "cxn", &mut connections);
        let mut points = Vec::new();
        visit(root, DGM, "pt", &mut points);
        let mut ids = BTreeSet::new();
        for p in &points {
            ensure!(
                ids.insert(
                    p.attr("modelId")
                        .context("SmartArt point missing modelId")?
                ),
                "duplicate SmartArt modelId at {part}"
            );
        }
        for connection in &connections {
            for key in ["srcId", "destId", "parTransId", "sibTransId"] {
                if let Some(id) = connection.attr(key) {
                    ensure!(
                        ids.contains(id),
                        "dangling SmartArt {key} at {part}/{}",
                        connection.attr("modelId").unwrap_or("?")
                    );
                } else {
                    ensure!(
                        !words("srcId destId", key),
                        "missing SmartArt {key} at {part}"
                    );
                }
            }
        }
        let mut hidden = BTreeSet::new();
        for point in points {
            let value = text(point);
            if value.is_empty() {
                continue;
            }
            let id = point.attr("modelId").unwrap();
            let mapped: Vec<_> = connections
                .iter()
                .filter(|c| {
                    c.attr("type") == Some("presOf")
                        && c.attr("srcId") == Some(id)
                        && c.attr("destId")
                            .is_some_and(|dest| cache.contains_key(dest))
                })
                .collect();
            if mapped.is_empty() {
                hidden.insert(id.to_owned());
                continue;
            }
            for cxn in mapped {
                let dest = cxn
                    .attr("destId")
                    .context("SmartArt presOf lacks destination")?;
                ensure!(
                    ids.contains(dest),
                    "SmartArt presOf destination absent at {part}/{id}"
                );
                let twin = cache.get(dest).with_context(|| {
                    format!("SmartArt point-to-cache mapping cannot be proved at {part}/{id}")
                })?;
                ensure!(
                    twin.is_empty() || value == *twin,
                    "inconsistent SmartArt model/cache text at {part}/{id}"
                );
            }
        }
        fn prune(
            n: &mut Node,
            hidden: &BTreeSet<String>,
            at: &str,
            report: &mut Report,
        ) -> Result<()> {
            let mut children = Vec::new();
            for mut c in std::mem::take(&mut n.children) {
                if c.ns == DGM
                    && ((c.local == "pt"
                        && c.attr("modelId").is_some_and(|id| hidden.contains(id)))
                        || (c.local == "cxn"
                            && ["srcId", "destId", "parTransId", "sibTransId"]
                                .iter()
                                .any(|k| c.attr(k).is_some_and(|id| hidden.contains(id)))))
                {
                    report.remove(
                        &format!("{at}/{}", c.attr("modelId").unwrap_or("?")),
                        "removed undrawn SmartArt data-model content and connections",
                        String::from_utf8(serialize(&c)?)?,
                    );
                    continue;
                }
                ensure!(
                    !c.attrs
                        .iter()
                        .any(|((_, k), v)| k == "presAssocID" && hidden.contains(v)),
                    "hidden SmartArt removal breaks display semantics at {at}"
                );
                prune(&mut c, hidden, at, report)?;
                children.push(c);
            }
            n.children = children;
            Ok(())
        }
        prune(trees.get_mut(&part).unwrap(), &hidden, &part, report)?;
        report.warnings.push(format!("{part}: SmartArt data-model and cached drawing text retained; review all text and alt text"));
    }
    Ok(())
}
fn definition(
    n: &mut Node,
    part: &str,
    urns: &BTreeMap<String, String>,
    report: &mut Report,
) -> Result<()> {
    let mut children = Vec::new();
    for mut c in std::mem::take(&mut n.children) {
        if c.ns == DGM && words("title desc catLst sampData styleData clrData", &c.local) {
            report.remove(
                part,
                "removed SmartArt definition free text",
                String::from_utf8(serialize(&c)?)?,
            );
            continue;
        }
        definition(&mut c, part, urns, report)?;
        children.push(c);
    }
    n.children = children;
    if n.ns == DGM && n.local == "shape" && n.attr_ns(R, "blip") == Some("") {
        n.attrs.remove(&(R.into(), "blip".into()));
    }
    for ((_, key), value) in &mut n.attrs {
        if n.ns == DGM && words("uniqueId loTypeId qsTypeId csTypeId presId", key) {
            if let Some(new) = urns.get(value) {
                report.remove(
                    &format!("{part}/@{key}"),
                    "neutralized SmartArt definition identifier/reference",
                    value.clone(),
                );
                *value = new.clone();
            }
        }
    }
    Ok(())
}
pub(super) fn location(parent: &str, n: &Node, index: usize) -> String {
    format!("{parent}/{{{}}}{}[{index}]", n.ns, n.local)
}
fn refs(n: &Node, out: &mut BTreeSet<String>) {
    for ((ns, _), v) in &n.attrs {
        if ns == R {
            out.insert(v.clone());
        }
    }
    for c in &n.children {
        refs(c, out);
    }
}
fn chartex(n: &Node, at: &str) -> Result<()> {
    ensure!(
        !(n.ns == MC
            && n.local == "Choice"
            && n.attr("Requires")
                .is_some_and(|v| v.contains("/chartex") || v.contains("/chartEx"))),
        "Office 2016 chartEx is unsupported at {at}"
    );
    ensure!(
        !n.ns.contains("/chartex") && !n.ns.contains("/chartEx"),
        "Office 2016 chartEx is unsupported at {at}"
    );
    for (i, c) in n.children.iter().enumerate() {
        chartex(c, &location(at, c, i + 1))?;
    }
    Ok(())
}
fn supported(n: &Node, part: &str) -> bool {
    if n.local == "absPath"
        && n.ns == "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac"
    {
        return true;
    }
    let mut trial = n.clone();
    let mut ignored = Report::default();
    cleanse(&mut trial, part, &mut ignored).is_ok() && validate_node(&trial, part).is_ok()
}
fn resolve_alternates(n: &mut Node, at: &str, report: &mut Report) -> Result<()> {
    let mut children = Vec::new();
    for (i, mut c) in std::mem::take(&mut n.children).into_iter().enumerate() {
        let loc = location(at, &c, i + 1);
        if c.ns == MC && c.local == "AlternateContent" {
            ensure!(
                c.children
                    .iter()
                    .all(|b| b.ns == MC && words("Choice Fallback", &b.local)),
                "invalid AlternateContent at {loc}"
            );
            let selected = c
                .children
                .iter()
                .position(|b| {
                    b.local == "Choice"
                        && b.attr("Requires").is_some_and(|v| {
                            v.split_ascii_whitespace().all(|ns| {
                                [
                                W,
                                S,
                                P,
                                A,
                                WP,
                                PIC,
                                XDR,
                                X14,
                                XM,
                                P14,
                                C,
                                CS,
                                DGM,
                                DSP,
                                "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac",
                                "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main",
                            ]
                            .contains(&ns)
                            })
                        })
                        && (!b.children.is_empty()
                            || c.children
                                .iter()
                                .filter(|b| b.local == "Choice")
                                .all(|b| b.children.iter().all(|n| n.local == "absPath")))
                        && b.children
                            .iter()
                            .all(|n| supported(n, at.split("/{").next().unwrap_or(at)))
                })
                .or_else(|| {
                    c.children.iter().position(|b| {
                        b.local == "Fallback"
                            && (!b.children.is_empty()
                                || c.children
                                    .iter()
                                    .filter(|b| b.local == "Choice")
                                    .all(|b| b.children.iter().all(|n| n.local == "absPath")))
                            && b.children
                                .iter()
                                .all(|n| supported(n, at.split("/{").next().unwrap_or(at)))
                    })
                });
            let selected = selected.with_context(|| {
                format!(
                    "no supported AlternateContent branch; visible content would be lost at {loc}"
                )
            })?;
            report.remove(
                &loc,
                "selected supported AlternateContent branch; removed alternate displayed content",
                String::from_utf8(serialize(&c)?)?,
            );
            let mut branch = c.children.remove(selected);
            resolve_alternates(&mut branch, &loc, report)?;
            children.extend(branch.children);
        } else {
            resolve_alternates(&mut c, &loc, report)?;
            children.push(c);
        }
    }
    n.children = children;
    Ok(())
}
fn filtered_series(n: &mut Node, at: &str, report: &mut Report) -> Result<()> {
    fn gather(n: &Node, hidden: &mut BTreeSet<u32>) -> Result<()> {
        if n.ns
            .starts_with("http://schemas.microsoft.com/office/drawing/")
            && n.ns.ends_with("/chart")
            && n.local.starts_with("filtered")
            && n.local.ends_with("Series")
        {
            let mut indices = Vec::new();
            visit(n, C, "idx", &mut indices);
            let idx = indices
                .first()
                .context("filtered chart series identity missing")?;
            hidden.insert(
                idx.attr("val")
                    .context("filtered chart series index missing")?
                    .parse()?,
            );
        }
        for c in &n.children {
            gather(c, hidden)?;
        }
        Ok(())
    }
    let mut hidden = BTreeSet::new();
    gather(n, &mut hidden)?;
    if hidden.is_empty() {
        return Ok(());
    }
    fn prune(n: &mut Node, ids: &BTreeSet<u32>, at: &str, report: &mut Report) -> Result<()> {
        let mut children = Vec::new();
        for mut c in std::mem::take(&mut n.children) {
            if c.ns == C && words("ser legendEntry", &c.local) {
                let idx = c
                    .children
                    .iter_mut()
                    .find(|c| c.ns == C && c.local == "idx")
                    .context("chart series identity missing")?;
                let index: u32 = idx
                    .attr("val")
                    .context("chart series index missing")?
                    .parse()?;
                if ids.contains(&index) {
                    report.remove(
                        at,
                        "removed filtered chart series and associated legend entry",
                        String::from_utf8(serialize(&c)?)?,
                    );
                    continue;
                }
                idx.attrs.insert(
                    (String::new(), "val".into()),
                    (index - ids.range(..index).count() as u32).to_string(),
                );
                if c.local == "ser" {
                    if let Some(order) = c
                        .children
                        .iter_mut()
                        .find(|c| c.ns == C && c.local == "order")
                    {
                        let old: u32 = order
                            .attr("val")
                            .context("chart series order missing")?
                            .parse()?;
                        order.attrs.insert(
                            (String::new(), "val".into()),
                            old.saturating_sub(ids.range(..old).count() as u32)
                                .to_string(),
                        );
                    }
                }
            }
            if c.local != "extLst" {
                prune(&mut c, ids, at, report)?;
            }
            children.push(c);
        }
        n.children = children;
        Ok(())
    }
    prune(n, &hidden, at, report)
}
fn filtered(n: &mut Node, at: &str, report: &mut Report) -> Result<()> {
    fn extension(n: &Node) -> bool {
        n.ns.starts_with("http://schemas.microsoft.com/office/drawing/") && n.ns.ends_with("/chart")
    }
    fn hidden_points(n: &Node, active: bool, ids: &mut BTreeSet<u32>) -> Result<()> {
        let active =
            active || (extension(n) && words("filteredLitCache filteredCategoryTitle", &n.local));
        if active && n.local == "pt" {
            ids.insert(
                n.attr("idx")
                    .context("filtered chart point lacks index")?
                    .parse()?,
            );
        }
        for c in &n.children {
            hidden_points(c, active, ids)?;
        }
        Ok(())
    }
    fn repair(n: &mut Node, ids: &BTreeSet<u32>, at: &str, report: &mut Report) -> Result<()> {
        let mut kept = Vec::new();
        for mut c in std::mem::take(&mut n.children) {
            if c.ns == C && words("pt dPt dLbl", &c.local) {
                let idx = if c.local == "pt" {
                    c.attr("idx")
                } else {
                    c.children
                        .iter()
                        .find(|x| x.ns == C && x.local == "idx")
                        .and_then(|x| x.attr("val"))
                };
                if let Some(idx) = idx {
                    let index: u32 = idx.parse()?;
                    if ids.contains(&index) {
                        report.remove(
                            at,
                            "removed filtered chart point/label/formatting",
                            String::from_utf8(serialize(&c)?)?,
                        );
                        continue;
                    }
                    let adjusted = index - ids.range(..index).count() as u32;
                    if c.local == "pt" {
                        c.attrs
                            .insert((String::new(), "idx".into()), adjusted.to_string());
                    } else if let Some(node) = c
                        .children
                        .iter_mut()
                        .find(|x| x.ns == C && x.local == "idx")
                    {
                        node.attrs
                            .insert((String::new(), "val".into()), adjusted.to_string());
                    }
                }
            }
            if c.ns == C && c.local == "ptCount" {
                let count: u32 = c.attr("val").context("chart cache count absent")?.parse()?;
                c.attrs.insert(
                    (String::new(), "val".into()),
                    (count - ids.range(..count).count() as u32).to_string(),
                );
            }
            if c.local != "extLst" && !(c.ns == C && c.local == "tx" && n.local == "ser") {
                repair(&mut c, ids, at, report)?;
            }
            kept.push(c);
        }
        n.children = kept;
        Ok(())
    }
    if n.ns == C && n.local == "ser" {
        let mut ids = BTreeSet::new();
        hidden_points(n, false, &mut ids)?;
        if !ids.is_empty() {
            repair(n, &ids, at, report)?;
        }
    }
    for (i, c) in n.children.iter_mut().enumerate() {
        filtered(c, &format!("{at}/point-scope[{i}]"), report)?;
    }
    Ok(())
}
fn chart(n: &mut Node, at: &str, report: &mut Report, series_text: bool) -> Result<()> {
    if n.ns == C && n.local == "chartSpace" {
        report.warnings.push(format!(
            "{at}: retained static chart caches may be stale; review every cached string and value"
        ));
    }
    ensure!(
        !(n.ns == C && n.local == "multiLvlStrRef"),
        "multi-level chart categories not supported; flatten in Excel or exclude at {at}"
    );
    if n.ns == C && n.local == "chart" {
        let setting = n
            .children
            .iter()
            .find(|c| c.ns == C && c.local == "plotVisOnly")
            .with_context(|| {
                format!("chart visibility setting missing; re-save in Office or exclude at {at}")
            })?;
        ensure!(
            !matches!(setting.attr("val"), Some("0" | "false")),
            "chart plots hidden rows or columns; unhide or exclude at {at}/plotVisOnly"
        );
        ensure!(
            setting.attr("val").is_none_or(|s| words("1 true", s)),
            "invalid chart visibility setting at {at}/plotVisOnly"
        );
    }
    if n.ns == C && words("strRef numRef", &n.local) {
        let cache = if n.local == "strRef" {
            "strCache"
        } else {
            "numCache"
        };
        let mut cached = n
            .children
            .iter()
            .find(|c| c.ns == C && c.local == cache)
            .cloned()
            .with_context(|| {
                format!("chart cache missing; recalculate/save locally or exclude at {at}")
            })?;
        ensure!(
            cached.children.iter().any(|c| c.local == "pt"),
            "chart cache missing; recalculate/save locally or exclude at {at}"
        );
        report.remove(
            at,
            "converted chart reference to static cache; caches may be stale",
            String::from_utf8(serialize(n)?)?,
        );
        if n.local == "strRef" && series_text {
            let points: Vec<_> = cached.children.iter().filter(|c| c.local == "pt").collect();
            ensure!(
                points.len() == 1,
                "chart text reference has ambiguous cache at {at}"
            );
            n.local = "v".into();
            n.attrs.clear();
            n.source_cell = points[0]
                .children
                .iter()
                .find(|c| c.local == "v")
                .and_then(|c| c.source_cell.clone());
            n.text = points[0].string();
            n.children.clear();
        } else {
            cached.local = if cache == "strCache" {
                "strLit"
            } else {
                "numLit"
            }
            .into();
            *n = cached;
        }
    }
    let mut children = Vec::new();
    for (i, mut c) in std::mem::take(&mut n.children).into_iter().enumerate() {
        let loc = location(at, &c, i + 1);
        if (c.ns==C && words("f externalData pivotSource pivotFmts printSettings headerFooter protection userShapes extLst",&c.local))
            || (c.ns==A && c.local=="fld" && c.attr("type")==Some("CELLRANGE")) {
            report.remove(&loc,if c.local=="fld" {"removed CELLRANGE displayed data label"} else {"removed chart source/extension content; retained static display only"},String::from_utf8(serialize(&c)?)?);
            continue;
        }
        chart(
            &mut c,
            &loc,
            report,
            n.ns == C && (n.local == "ser" || (n.local == "tx" && series_text)),
        )?;
        children.push(c);
    }
    n.children = children;
    Ok(())
}
/// Resolve visible branches before applying binary/embedding refusal. Only parts
/// reached solely through deleted dependencies may be collected.
pub(super) fn prepare(parts: &mut BTreeMap<String, Vec<u8>>, report: &mut Report) -> Result<()> {
    for (part, bytes) in parts
        .iter()
        .filter(|(p, _)| p.ends_with(".xml") && !p.contains("/charts/") && !removable(p))
    {
        chartex(&parse(bytes, part)?, part)?;
    }
    for (part, bytes) in parts.iter() {
        let lower = part.to_ascii_lowercase();
        ensure!(
            !lower.contains("vbaproject")
                && !lower.contains("/activex/")
                && !lower.starts_with("_xmlsignatures/")
                && !bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]),
            "VBA/ActiveX/OLE/signature refused at {part}"
        );
    }
    let sheets = super::chart_sources::sheets(parts)?;
    let mut trees = BTreeMap::new();
    let mut deleted = BTreeMap::<String, BTreeSet<String>>::new();
    for (part, bytes) in parts.iter() {
        if !part.ends_with(".xml") || removable(part) {
            continue;
        }
        let mut n = parse(bytes, part)?;
        chartex(&n, part)?;
        let mut before = BTreeSet::new();
        refs(&n, &mut before);
        resolve_alternates(&mut n, part, report)?;
        if n.ns == C {
            super::chart_sources::annotate(&mut n, part, &sheets, &mut report.chart_sources)?;
            filtered_series(&mut n, part, report)?;
            filtered(&mut n, part, report)?;
            chart(&mut n, part, report, false)?;
        }
        let mut after = BTreeSet::new();
        refs(&n, &mut after);
        deleted.insert(part.clone(), before.difference(&after).cloned().collect());
        trees.insert(part.clone(), n);
    }
    let mut candidates = BTreeSet::new();
    let mut rels = BTreeMap::new();
    for (part, bytes) in parts.iter().filter(|(p, _)| p.ends_with(".rels")) {
        let own = owner(part)?;
        let mut n = parse(bytes, part)?;
        let mut keep = Vec::new();
        for rel in std::mem::take(&mut n.children) {
            let kind = rel
                .attr("Type")
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("");
            let drop = deleted
                .get(&own)
                .is_some_and(|ids| rel.attr("Id").is_some_and(|id| ids.contains(id)))
                || (trees.get(&own).is_some_and(|n| n.ns == C) && kind == "themeOverride");
            if drop {
                ensure!(
                    !words("oleObject control vbaProject", kind),
                    "unrelated OLE/active/embedded content at {part}"
                );
                if kind == "package" {
                    ensure!(
                        trees.get(&own).is_some_and(|n| n.ns == C),
                        "unproven chart workbook dependency at {part}"
                    );
                    if rel.attr("TargetMode") != Some("External") {
                        let target = resolve(
                            &own,
                            rel.attr("Target").context("chart workbook target absent")?,
                        )?;
                        ensure!(
                            target.ends_with(".xlsx")
                                && parts
                                    .get(&target)
                                    .is_some_and(|b| b.starts_with(b"PK\x03\x04")),
                            "unproven chart workbook dependency at {part}"
                        );
                    }
                }
                if kind == "themeOverride" {
                    report.warnings.push(format!(
                        "{own}: removing themeOverride may change chart colours"
                    ));
                }
                report.remove(
                    part,
                    "removed unused graphics relationship",
                    String::from_utf8(serialize(&rel)?)?,
                );
                if rel.attr("TargetMode") != Some("External") {
                    candidates.insert(resolve(
                        &own,
                        rel.attr("Target").context("graphics target missing")?,
                    )?);
                }
            } else {
                keep.push(rel);
            }
        }
        n.children = keep;
        rels.insert(part.clone(), n);
    }
    loop {
        let incoming: BTreeSet<_> = rels
            .iter()
            .flat_map(|(part, n)| {
                n.children
                    .iter()
                    .filter(|r| r.attr("TargetMode") != Some("External"))
                    .map(move |r| {
                        resolve(&owner(part)?, r.attr("Target").context("target absent")?)
                    })
            })
            .collect::<Result<_>>()?;
        let orphaned: Vec<_> = candidates
            .iter()
            .filter(|p| !incoming.contains(*p))
            .cloned()
            .collect();
        if orphaned.is_empty() {
            break;
        }
        for p in orphaned {
            candidates.remove(&p);
            if let Some(bytes) = parts.remove(&p) {
                report.remove(
                    &p,
                    "removed orphaned graphics dependency",
                    String::from_utf8_lossy(&bytes).into_owned(),
                );
            }
            trees.remove(&p);
            let rp = rels
                .iter()
                .find(|(rp, _)| owner(rp).ok().as_ref() == Some(&p))
                .map(|(p, _)| p.clone());
            if let Some(rp) = rp {
                let n = rels.remove(&rp).unwrap();
                parts.remove(&rp);
                for r in n.children {
                    if r.attr("TargetMode") != Some("External") {
                        candidates.insert(resolve(&p, r.attr("Target").context("target absent")?)?);
                    }
                }
            }
        }
    }
    diagram_models(&mut trees, &rels, report)?;
    let urns: BTreeMap<_, _> = trees
        .values()
        .filter(|n| n.ns == DGM && words("layoutDef styleDef colorsDef", &n.local))
        .filter_map(|n| n.attr("uniqueId"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(i, s)| (s.to_owned(), format!("urn:vectoraiz:diagram:{i}")))
        .collect();
    for (part, n) in &mut trees {
        definition(n, part, &urns, report)?;
    }
    // Pruned declarations cannot survive into the generic dangling-type check.
    if let Some(ct) = trees.get_mut("[Content_Types].xml") {
        ct.children.retain(|n| {
            n.local != "Override"
                || n.attr("PartName")
                    .is_some_and(|p| parts.contains_key(p.trim_start_matches('/')))
        });
    }
    for (part, mut n) in trees.into_iter().chain(rels) {
        if n.ns == S && n.local == "worksheet" {
            inspect_formulas(&n, &part, report)?;
        }
        cleanse(&mut n, &part, report)?;
        super::chart_sources::collect(&n, &part, &mut report.chart_sources);
        parts.insert(part, serialize(&n)?);
    }
    Ok(())
}

/// Expand an edit into both representations using the same selectors as walk.
pub(super) fn paired_edits(
    trees: &BTreeMap<String, Node>,
    edits: &BTreeMap<String, Edit>,
) -> Result<BTreeMap<String, Edit>> {
    let mut result = edits.clone();
    if edits.is_empty() {
        return Ok(result);
    }
    fn paths(
        n: &Node,
        at: &str,
        out: &mut BTreeMap<(String, String), Vec<(String, String)>>,
        current: Option<(String, String)>,
    ) {
        let current = if n.ns == DGM && n.local == "pt" {
            n.attr("modelId").map(|id| (DGM.to_owned(), id.to_owned()))
        } else if n.ns == DSP && n.local == "sp" {
            n.attr("modelId").map(|id| (DSP.to_owned(), id.to_owned()))
        } else {
            current
        };
        if n.ns == A && n.local == "t" {
            if let Some(key) = &current {
                out.entry(key.clone())
                    .or_default()
                    .push((format!("{at}/text"), n.text.clone()));
            }
        }
        let mut counts = BTreeMap::new();
        for c in &n.children {
            let i = counts.entry((&c.ns, &c.local)).or_insert(0);
            *i += 1;
            paths(c, &location(at, c, *i), out, current.clone());
        }
    }
    let rels = trees
        .iter()
        .filter(|(p, _)| p.ends_with(".rels"))
        .map(|(p, n)| (p.clone(), n.clone()))
        .collect();
    loop {
        let previous = result.len();
        for (part, data) in trees
            .iter()
            .filter(|(_, n)| n.ns == DGM && n.local == "dataModel")
        {
            let target = drawing_for(part, data, trees, &rels)?;
            let mut nodes = BTreeMap::new();
            paths(data, part, &mut nodes, None);
            paths(&trees[&target], &target, &mut nodes, None);
            let mut connections = Vec::new();
            visit(data, DGM, "cxn", &mut connections);
            for cxn in connections
                .iter()
                .filter(|n| n.attr("type") == Some("presOf"))
            {
                let model =
                    nodes.get(&(DGM.to_owned(), cxn.attr("srcId").unwrap_or("").to_owned()));
                let drawing =
                    nodes.get(&(DSP.to_owned(), cxn.attr("destId").unwrap_or("").to_owned()));
                if let (Some(model), Some(drawing)) = (model, drawing) {
                    let touched = model
                        .iter()
                        .chain(drawing)
                        .any(|(at, _)| result.contains_key(at));
                    if !touched {
                        continue;
                    }
                    ensure!(
                        model.len() == drawing.len()
                            && model.iter().zip(drawing).all(|(a, b)| a.1 == b.1),
                        "SmartArt paired edit cannot prove run mapping at {part}"
                    );
                    for (a, b) in model.iter().zip(drawing) {
                        if let Some(edit) = result.get(&a.0).or_else(|| result.get(&b.0)).cloned() {
                            for (at, prior) in [a, b] {
                                ensure!(
                                    crate::pipeline::sha256(prior.as_bytes())
                                        == edit.expected_sha256,
                                    "SmartArt prior-value mismatch at {at}"
                                );
                                ensure!(
                                    result.get(at).is_none_or(|e| e.value == edit.value),
                                    "conflicting SmartArt twin edit at {at}"
                                );
                                result.insert(at.clone(), edit.clone());
                            }
                        }
                    }
                }
            }
        }
        if result.len() == previous {
            break;
        }
    }
    Ok(result)
}
/// Independent model/cache proof; readback must never discover removable points.
pub(super) fn validate_models(trees: &BTreeMap<String, Node>) -> Result<()> {
    if !trees
        .values()
        .any(|n| n.ns == DGM && n.local == "dataModel")
    {
        return Ok(());
    }
    let rels = trees
        .iter()
        .filter(|(p, _)| p.ends_with(".rels"))
        .map(|(p, n)| (p.clone(), n.clone()))
        .collect();
    let mut staged = trees.clone();
    let mut report = Report::default();
    diagram_models(&mut staged, &rels, &mut report)?;
    ensure!(
        report.removals.is_empty(),
        "undrawn SmartArt point survived readback"
    );
    Ok(())
}

pub(super) fn validate_links(trees: &BTreeMap<String, Node>) -> Result<()> {
    fn check(n: &Node, part: &str, trees: &BTreeMap<String, Node>) -> Result<()> {
        let links: Vec<_> = if n.ns == C && n.local == "chart" && n.attr_ns(R, "id").is_some() {
            vec![("id", "chart", C, "chartSpace")]
        } else if n.ns == DGM && n.local == "relIds" {
            vec![
                ("dm", "diagramData", DGM, "dataModel"),
                ("lo", "diagramLayout", DGM, "layoutDef"),
                ("qs", "diagramQuickStyle", DGM, "styleDef"),
                ("cs", "diagramColors", DGM, "colorsDef"),
            ]
        } else {
            vec![]
        };
        for (key, kind, ns, local) in links {
            let id = n
                .attr_ns(R, key)
                .with_context(|| format!("missing graphics relationship at {part}/@{key}"))?;
            let rel = trees
                .get(&relpath(part)?)
                .and_then(|rs| rs.children.iter().find(|r| r.attr("Id") == Some(id)))
                .with_context(|| format!("missing graphics relationship at {part}/{id}"))?;
            ensure!(
                rel.attr("Type") == Some(&format!("{R}/{kind}"))
                    && rel.attr("TargetMode") != Some("External"),
                "mismatched graphics relationship at {part}/{id}"
            );
            let target = resolve(part, rel.attr("Target").context("graphics target absent")?)?;
            ensure!(
                trees
                    .get(&target)
                    .is_some_and(|root| root.ns == ns && root.local == local),
                "mismatched graphics target at {part}/{id}"
            );
        }
        for c in &n.children {
            check(c, part, trees)?;
        }
        Ok(())
    }
    for (part, n) in trees {
        if !part.ends_with(".rels") {
            check(n, part, trees)?;
        }
    }
    Ok(())
}
