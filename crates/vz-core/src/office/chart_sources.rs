//! Source-to-static-cache bindings are private review evidence, never OOXML.
use super::*;
use graphics::C;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartSources {
    pub charts: BTreeSet<String>,
    pub unmappable: BTreeSet<String>,
    pub points: Vec<CacheSource>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheSource {
    pub sheet: String,
    pub cell: String,
    pub target: String,
    pub numeric: bool,
}

pub(super) fn sheets(parts: &BTreeMap<String, Vec<u8>>) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    let (Some(workbook), Some(rels)) = (
        parts.get("xl/workbook.xml"),
        parts.get("xl/_rels/workbook.xml.rels"),
    ) else {
        return Ok(result);
    };
    let workbook = parse(workbook, "xl/workbook.xml")?;
    let rels = parse(rels, "xl/_rels/workbook.xml.rels")?;
    for sheet in workbook
        .children
        .iter()
        .filter(|n| n.ns == S && n.local == "sheets")
        .flat_map(|n| &n.children)
    {
        let (Some(name), Some(id)) = (sheet.attr("name"), sheet.attr_ns(R, "id")) else {
            continue;
        };
        let Some(rel) = rels.children.iter().find(|n| {
            n.attr("Id") == Some(id)
                && n.attr("Type") == Some(&format!("{R}/worksheet"))
                && n.attr("TargetMode") != Some("External")
        }) else {
            continue;
        };
        if let Some(target) = rel
            .attr("Target")
            .and_then(|t| resolve("xl/workbook.xml", t).ok())
        {
            if result.insert(name.to_lowercase(), target).is_some() {
                result.clear();
                return Ok(result);
            }
        }
    }
    Ok(result)
}
fn cell(s: &str) -> Option<(u32, u32)> {
    let upper = s.to_ascii_uppercase();
    let s = upper.strip_prefix('$').unwrap_or(&upper);
    let split = s.find(|c: char| !c.is_ascii_uppercase())?;
    let (col, row) = s.split_at(split);
    let row = row.strip_prefix('$').unwrap_or(row);
    if col.is_empty()
        || !col.bytes().all(|b| b.is_ascii_uppercase())
        || !row.bytes().all(|b| b.is_ascii_digit())
        || row.starts_with('0')
    {
        return None;
    }
    let col = col.bytes().try_fold(0u32, |n, b| {
        n.checked_mul(26)?.checked_add(u32::from(b - b'A' + 1))
    })?;
    let row = row.parse::<u32>().ok()?;
    (col <= 16384 && row <= 1048576).then_some((col, row))
}
fn address(mut col: u32, row: u32) -> String {
    let mut s = Vec::new();
    while col > 0 {
        col -= 1;
        s.push((b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    format!("{}{row}", s.into_iter().rev().collect::<String>())
}
struct SourceRange {
    sheet: String,
    first: (u32, u32),
    last: (u32, u32),
}
fn range(formula: &str, sheets: &BTreeMap<String, String>) -> Option<SourceRange> {
    let (sheet, range) = formula.rsplit_once('!')?;
    let sheet = if sheet.starts_with('\'') && sheet.ends_with('\'') {
        if sheet.len() < 2 {
            return None;
        }
        let inner = &sheet[1..sheet.len() - 1];
        if inner.replace("''", "").contains('\'') {
            return None;
        }
        inner.replace("''", "'")
    } else {
        if !sheet
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return None;
        }
        sheet.into()
    };
    let sheet = sheets.get(&sheet.to_lowercase())?.clone();
    let (first, last) = range.split_once(':').unwrap_or((range, range));
    let (first, last) = (cell(first)?, cell(last)?);
    if first.0 > last.0 || first.1 > last.1 || (first.0 != last.0 && first.1 != last.1) {
        return None;
    }
    Some(SourceRange { sheet, first, last })
}
pub(super) fn annotate(
    n: &mut Node,
    part: &str,
    sheets: &BTreeMap<String, String>,
    out: &mut ChartSources,
) -> Result<()> {
    if !part.starts_with("xl/") {
        return Ok(());
    }
    let chart = format!("{part}/{{{C}}}chart[1]");
    out.charts.insert(chart.clone());
    fn mark(n: &mut Node, sheets: &BTreeMap<String, String>, bad: &mut bool) {
        if n.ns == C && words("strRef numRef", &n.local) {
            let formulas: Vec<_> = n
                .children
                .iter()
                .filter(|n| n.ns == C && n.local == "f")
                .collect();
            let parsed = if formulas.len() == 1 {
                range(&formulas[0].text, sheets)
            } else {
                None
            };
            let Some(SourceRange { sheet, first, last }) = parsed else {
                *bad = true;
                return;
            };
            let count = last.0 - first.0 + last.1 - first.1 + 1;
            let mut valid = false;
            for cache in n
                .children
                .iter_mut()
                .filter(|c| c.ns == C && words("strCache numCache", &c.local))
            {
                if cache
                    .children
                    .iter()
                    .find(|c| c.ns == C && c.local == "ptCount")
                    .and_then(|n| n.attr("val"))
                    .and_then(|s| s.parse::<u32>().ok())
                    != Some(count)
                {
                    *bad = true;
                    continue;
                }
                let mut seen = BTreeSet::new();
                for pt in cache
                    .children
                    .iter_mut()
                    .filter(|n| n.ns == C && n.local == "pt")
                {
                    let Some(idx) = pt
                        .attr("idx")
                        .and_then(|s| s.parse::<u32>().ok())
                        .filter(|i| *i < count && seen.insert(*i))
                    else {
                        *bad = true;
                        continue;
                    };
                    let col = first.0 + if first.0 == last.0 { 0 } else { idx };
                    let row = first.1 + if first.1 == last.1 { 0 } else { idx };
                    let values: Vec<_> = pt
                        .children
                        .iter_mut()
                        .filter(|c| c.ns == C && c.local == "v")
                        .collect();
                    if values.len() != 1 {
                        *bad = true;
                        continue;
                    }
                    for v in values {
                        v.source_cell = Some(Box::new((sheet.clone(), address(col, row))));
                        valid = true;
                    }
                }
            }
            if !valid {
                *bad = true;
            }
            return;
        }
        if n.ns == C && n.local == "f" {
            *bad = true;
        }
        for c in &mut n.children {
            mark(c, sheets, bad);
        }
    }
    let mut bad = false;
    mark(n, sheets, &mut bad);
    if bad {
        out.unmappable.insert(chart);
    }
    Ok(())
}
pub(super) fn collect(n: &Node, at: &str, out: &mut ChartSources) {
    fn visit(n: &Node, at: &str, numeric: bool, out: &mut ChartSources) {
        let numeric = numeric || n.ns == C && n.local == "numLit";
        if let Some((sheet, cell)) = n.source_cell.as_deref() {
            out.points.push(CacheSource {
                sheet: sheet.clone(),
                cell: cell.clone(),
                target: format!("{at}/text"),
                numeric,
            });
        }
        let mut counts = BTreeMap::new();
        for c in &n.children {
            let i = counts.entry((&c.ns, &c.local)).or_insert(0);
            *i += 1;
            visit(c, &graphics::location(at, c, *i), numeric, out);
        }
    }
    visit(n, at, false, out);
}
fn numeric_cells<'a>(n: &'a Node, part: &'a str, out: &mut BTreeSet<(&'a str, &'a str)>) {
    if n.ns == S && n.local == "c" && n.attr("t").is_none_or(|t| t == "n") {
        if let Some(cell) = n.attr("r") {
            out.insert((part, cell));
        }
    }
    for c in &n.children {
        numeric_cells(c, part, out);
    }
}
impl ChartSources {
    pub(super) fn rename(&mut self, rename: &BTreeMap<String, String>) {
        fn path(s: &str, rename: &BTreeMap<String, String>) -> String {
            let (part, tail) = s.split_once("/{").unwrap_or((s, ""));
            let part = rename.get(part).map(String::as_str).unwrap_or(part);
            if tail.is_empty() {
                part.into()
            } else {
                format!("{part}/{{{tail}")
            }
        }
        self.charts = self.charts.iter().map(|s| path(s, rename)).collect();
        self.unmappable = self.unmappable.iter().map(|s| path(s, rename)).collect();
        for p in &mut self.points {
            p.sheet = rename.get(&p.sheet).unwrap_or(&p.sheet).clone();
            p.target = path(&p.target, rename);
        }
    }
    pub(super) fn expand(
        &self,
        trees: &BTreeMap<String, Node>,
        edits: &BTreeMap<String, Edit>,
    ) -> Result<BTreeMap<String, Edit>> {
        if let Some(key) = edits
            .keys()
            .find(|k| k.contains(&format!("{{{C}}}numLit[")) && k.ends_with("/text"))
        {
            bail!("text edit lands on numeric chart cache point at {key}");
        }
        let mut result = edits.clone();
        let cell_edits: Vec<_> = edits
            .iter()
            .filter(|(k, _)| k.starts_with("xl/") && k.contains("/cell/"))
            .collect();
        if cell_edits.is_empty() {
            return Ok(result);
        }
        let mut chart_count = 0;
        for (part, _) in trees
            .iter()
            .filter(|(p, n)| p.starts_with("xl/") && n.ns == C && n.local == "chartSpace")
        {
            chart_count += 1;
            let at = format!("{part}/{{{C}}}chart[1]");
            ensure!(
                self.charts.contains(&at) && !self.unmappable.contains(&at),
                "XLSX cell edit cannot prove chart cache-to-source mapping at {at}"
            );
        }
        if chart_count == 0 {
            return Ok(result);
        }
        let mut report = Report::default();
        for (part, tree) in trees {
            walk(
                tree.clone(),
                part,
                &mut report,
                &BTreeMap::new(),
                &mut BTreeSet::new(),
            )?;
        }
        let values: BTreeMap<_, _> = report
            .inventory
            .iter()
            .filter_map(|v| v.value.as_deref().map(|s| (v.location.as_str(), s)))
            .collect();
        let mut cell_values: BTreeMap<(&str, &str), Vec<&Value>> = BTreeMap::new();
        for v in report
            .inventory
            .iter()
            .filter(|v| v.location.ends_with("/text"))
        {
            if let Some((prefix, tail)) = v.location.split_once("/cell/") {
                if let (Some((part, _)), Some((cell, _))) =
                    (prefix.split_once("/{"), tail.split_once('/'))
                {
                    cell_values.entry((part, cell)).or_default().push(v);
                }
            }
        }
        let mut numeric = BTreeSet::new();
        for (part, n) in trees {
            numeric_cells(n, part, &mut numeric);
        }
        let mut points: BTreeMap<(&str, &str), Vec<&CacheSource>> = BTreeMap::new();
        for point in &self.points {
            points
                .entry((&point.sheet, &point.cell))
                .or_default()
                .push(point);
        }
        for (key, edit) in cell_edits {
            let (part, tail) = key.split_once("/{").context("invalid cell edit selector")?;
            let (_, tail) = tail
                .split_once("/cell/")
                .context("invalid cell edit selector")?;
            let (cell, _) = tail.split_once('/').context("invalid cell edit selector")?;
            for point in points.get(&(part, cell)).into_iter().flatten() {
                let source = cell_values
                    .get(&(part, cell))
                    .context("chart source cell missing")?;
                ensure!(source.len()==1 && source[0].location==*key,"XLSX cell edit cannot prove chart cache-to-source mapping for split/duplicate cell at {}",point.target);
                let value = values.get(point.target.as_str()).with_context(|| {
                    format!("XLSX chart mapping target missing at {}", point.target)
                })?;
                ensure!(
                    !point.numeric
                        || (numeric.contains(&(part, cell))
                            && key.contains(&format!("{{{S}}}v["))
                            && plain_number(&edit.value)),
                    "text edit lands on numeric chart cache point at {}",
                    point.target
                );
                ensure!(
                    result
                        .get(&point.target)
                        .is_none_or(|e| e.value == edit.value),
                    "conflicting chart/cache edits at {}",
                    point.target
                );
                if let Some(requested) = result.get(&point.target) {
                    ensure!(
                        requested.expected_sha256 == crate::pipeline::sha256(value.as_bytes()),
                        "Office prior-value mismatch at {}",
                        point.target
                    );
                }
                result.insert(
                    point.target.clone(),
                    Edit {
                        expected_sha256: crate::pipeline::sha256(value.as_bytes()),
                        value: edit.value.clone(),
                    },
                );
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_provable_local_linear_a1_ranges() {
        let sheets = BTreeMap::from([
            ("sheet1".into(), "xl/worksheets/source.xml".into()),
            ("o'brien".into(), "xl/worksheets/quoted.xml".into()),
        ]);
        for formula in [
            "Sheet1!$A$1:$A$3",
            "Sheet1!A1:C1",
            "sheet1!a1:b1",
            "Sheet1!B2",
            "'O''Brien'!$XFD$1048576",
        ] {
            assert!(range(formula, &sheets).is_some(), "{formula}");
        }
        for formula in [
            "DefinedName",
            "'!A1",
            "SUM(Sheet1!A1)",
            "Sheet1!A1:B2",
            "Sheet1!B2:A1",
            "[book.xlsx]Sheet1!A1",
            "Sheet1!$$A1",
            "Sheet1!A$1$",
            "Sheet1!A0",
            "Sheet1!XFE1",
            "Sheet1!A1048577",
            "Sheet1!A1,Sheet1!A2",
            "'O'Brien'!A1",
        ] {
            assert!(range(formula, &sheets).is_none(), "{formula}");
        }
    }
}
