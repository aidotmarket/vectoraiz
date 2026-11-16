//! Static source caches only. No Office rebuilding, workbook evaluation or render.
use super::super::graphics::{C, DGM, DSP};
use super::*;

pub(super) fn reference(
    n: &Node,
    host: &str,
    at: &str,
    hidden: bool,
    trees: &BTreeMap<String, Node>,
    out: &mut Projection,
) -> Result<bool> {
    if n.ns == C && n.local == "chart" {
        let part = resolve(
            host,
            linked(
                trees,
                host,
                n.attr_ns(R, "id").context("chart relationship")?,
            )?,
        )?;
        let root = trees.get(&part).context("missing chart")?;
        ensure!(
            root.ns == C && root.local == "chartSpace",
            "unknown chart root at {part}"
        );
        chart(root, &format!("{at}!{part}"), hidden, out, false)?;
        out.warnings.push(format!(
            "{at}: static chart caches only; no recalculation or rendering"
        ));
        return Ok(true);
    }
    if n.ns == DGM && n.local == "relIds" {
        let data = resolve(
            host,
            linked(
                trees,
                host,
                n.attr_ns(R, "dm").context("SmartArt model relationship")?,
            )?,
        )?;
        diagram(&data, host, at, hidden, trees, out)?;
        out.warnings.push(format!(
            "{at}: static SmartArt model/cache pairs; no layout rendering"
        ));
        return Ok(true);
    }
    Ok(false)
}
fn chart(n: &Node, at: &str, hidden: bool, out: &mut Projection, cache: bool) -> Result<()> {
    ensure!(
        n.ns == C || n.ns == A,
        "PROJECTION_CHART_SEMANTICS_UNSUPPORTED at {at}: {}",
        n.ns
    );
    ensure!(
        n.text.trim().is_empty()
            || (n.ns == A && n.local == "t")
            || (n.ns == C && ["f", "v", "formatCode"].contains(&n.local.as_str())),
        "PROJECTION_CHART_SEMANTICS_UNSUPPORTED at {at}: unknown text carrier"
    );
    ensure!(
        !(n.ns == C && ["multiLvlStrRef", "multiLvlStrCache"].contains(&n.local.as_str())),
        "PROJECTION_CHART_SEMANTICS_UNSUPPORTED at {at}: multilevel cache completeness"
    );
    if n.ns == C && n.local == "f" {
        return out.drop_at("formulas", at);
    }
    if n.ns == C && ["strRef", "numRef", "multiLvlStrRef"].contains(&n.local.as_str()) {
        let required = match n.local.as_str() {
            "strRef" => "strCache",
            "numRef" => "numCache",
            _ => "multiLvlStrCache",
        };
        ensure!(n.children.iter().filter(|c|c.ns==C && c.local==required).count()==1,"PROJECTION_MISSING_CACHE at {at}: required {required}; recalculate/save locally or exclude document");
    }
    let cache = cache
        || (n.ns == C
            && [
                "strCache",
                "numCache",
                "multiLvlStrCache",
                "strLit",
                "numLit",
                "tx",
            ]
            .contains(&n.local.as_str()));
    if n.ns == C && ["strCache", "numCache", "strLit", "numLit"].contains(&n.local.as_str()) {
        let count: usize = n
            .children
            .iter()
            .find(|c| c.ns == C && c.local == "ptCount")
            .and_then(|c| c.attr("val"))
            .context("chart point count missing")?
            .parse()?;
        let points: Vec<_> = n
            .children
            .iter()
            .filter(|c| c.ns == C && c.local == "pt")
            .collect();
        ensure!(
            points.len() == count && count > 0,
            "PROJECTION_MISSING_CACHE at {at}: missing/sparse chart cache"
        );
        let mut indexes = BTreeSet::new();
        for p in points {
            let idx: usize = p.attr("idx").context("chart point index")?.parse()?;
            ensure!(
                idx < count && indexes.insert(idx),
                "ambiguous chart cache at {at}"
            );
            ensure!(
                p.children
                    .iter()
                    .filter(|c| c.ns == C && c.local == "v")
                    .count()
                    == 1,
                "missing chart cached value at {at}"
            );
        }
    }
    if (n.ns == A && n.local == "t") || (n.ns == C && n.local == "v") {
        ensure!(
            n.ns == A || cache,
            "PROJECTION_CHART_SEMANTICS_UNSUPPORTED at {at}: value outside static cache"
        );
        out.add(at, "chart cached value", n.text.clone(), hidden)?;
    }
    // User-facing attributes are content; rendering properties are never text.
    if n.ns == A && n.local == "cNvPr" {
        for key in ["title", "descr"] {
            if let Some(value) = n.attr(key) {
                out.add(&format!("{at}/@{key}"), "alt text", value.into(), hidden)?;
            }
        }
    }
    ensure!(
        !["ext", "AlternateContent", "userShapes"].contains(&n.local.as_str()),
        "PROJECTION_CHART_SEMANTICS_UNSUPPORTED at {at}"
    );
    for (i, c) in n.children.iter().enumerate() {
        chart(
            c,
            &format!("{at}/{}[{}]", c.local, i + 1),
            hidden,
            out,
            cache,
        )?;
    }
    Ok(())
}
fn diagram(
    data: &str,
    host: &str,
    at: &str,
    hidden: bool,
    trees: &BTreeMap<String, Node>,
    out: &mut Projection,
) -> Result<()> {
    let root = trees.get(data).context("SmartArt model missing")?;
    ensure!(
        root.ns == DGM && root.local == "dataModel",
        "SmartArt model root unsupported at {data}"
    );
    let mut exts = Vec::new();
    descendants(root, DSP, "dataModelExt", &mut exts);
    ensure!(
        exts.len() == 1,
        "PROJECTION_MISSING_CACHE at {data}: SmartArt drawing cache absent"
    );
    let id = exts[0]
        .attr("relId")
        .context("SmartArt drawing relationship")?;
    let mut targets = BTreeSet::new();
    for own in [data, host] {
        if let Ok(target) = linked(trees, own, id) {
            let target = resolve(own, target)?;
            if trees
                .get(&target)
                .is_some_and(|n| n.ns == DSP && n.local == "drawing")
            {
                targets.insert(target);
            }
        }
    }
    ensure!(
        targets.len() == 1,
        "PROJECTION_MISSING_CACHE at {data}: ambiguous/missing SmartArt drawing cache"
    );
    let drawing = targets.into_iter().next().unwrap();
    let mut shapes = Vec::new();
    descendants(&trees[&drawing], DSP, "sp", &mut shapes);
    let mut cache = BTreeMap::new();
    for shape in shapes {
        let key = shape.attr("modelId").context("SmartArt shape modelId")?;
        ensure!(
            cache
                .insert(key, (drawing_text(shape), own_hidden_shape(shape)?))
                .is_none(),
            "ambiguous SmartArt shape at {drawing}/{key}"
        );
    }
    let mut points = Vec::new();
    descendants(root, DGM, "pt", &mut points);
    let mut connections = Vec::new();
    descendants(root, DGM, "cxn", &mut connections);
    let mut used = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for point in points {
        let id = point.attr("modelId").context("SmartArt point modelId")?;
        ensure!(ids.insert(id), "duplicate SmartArt modelId at {data}");
        let value = drawing_text(point);
        if value.trim().is_empty() {
            continue;
        }
        // Prefer the explicit presentation pairing; the source model itself
        // can also be the cached shape's modelId when no presOf edge exists.
        let linked: Vec<_> = connections
            .iter()
            .filter(|c| c.attr("type") == Some("presOf") && c.attr("srcId") == Some(id))
            .filter_map(|c| c.attr("destId"))
            .collect();
        let cache_id = if linked.is_empty() {
            id
        } else {
            ensure!(
                linked.len() == 1,
                "ambiguous SmartArt pairing at {data}/{id}"
            );
            linked[0]
        };
        let (displayed, shape_hidden) = cache
            .get(cache_id)
            .context("SmartArt text lacks paired static cache")?;
        ensure!(
            *displayed == value,
            "PROJECTION_SMARTART_PAIR_AMBIGUOUS at {data}/{id}: model/cache disagree"
        );
        ensure!(used.insert(cache_id), "PROJECTION_SMARTART_PAIR_AMBIGUOUS at {data}/{id}: multiple model texts share one cache");
        out.add(
            &format!("{at}!{data}/model/{id}!{drawing}/shape/{cache_id}"),
            "SmartArt paired text",
            value,
            hidden || *shape_hidden,
        )?;
    }
    for (id, (value, _)) in cache {
        ensure!(
            value.trim().is_empty() || used.contains(id),
            "PROJECTION_SMARTART_PAIR_AMBIGUOUS at {drawing}/{id}: unpaired cache text"
        );
    }
    Ok(())
}
fn drawing_text(n: &Node) -> String {
    let mut nodes = Vec::new();
    descendants(n, A, "t", &mut nodes);
    nodes
        .iter()
        .map(|n| n.text.as_str())
        .collect::<Vec<_>>()
        .join("")
}
