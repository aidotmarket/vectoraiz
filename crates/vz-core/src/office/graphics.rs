//! Static graphics preparation. Dependency deletion is scoped to removed references.
use super::*;
pub(super) const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
pub(super) const CS: &str = "http://schemas.microsoft.com/office/drawing/2012/chartStyle";
fn location(parent: &str, n: &Node, index: usize) -> String {
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
        !n.ns.contains("/chartex") && !n.ns.contains("/chartEx"),
        "Office 2016 chartEx is unsupported at {at}"
    );
    for (i, c) in n.children.iter().enumerate() {
        chartex(c, &location(at, c, i + 1))?;
    }
    Ok(())
}
fn supported(n: &Node, part: &str) -> bool {
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
fn chart(n: &mut Node, at: &str, report: &mut Report) -> Result<()> {
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
        if n.local == "strRef" && at.contains("}tx[") {
            let points: Vec<_> = cached.children.iter().filter(|c| c.local == "pt").collect();
            ensure!(
                points.len() == 1,
                "chart text reference has ambiguous cache at {at}"
            );
            n.local = "v".into();
            n.attrs.clear();
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
        chart(&mut c, &loc, report)?;
        children.push(c);
    }
    n.children = children;
    if n.ns == C
        && n.local == "tx"
        && at.contains("}title[")
        && n.children.len() == 1
        && n.children[0].local == "v"
    {
        let value = n.children.remove(0).text;
        let leaf = |ns: &str, local: &str, text: String, children: Vec<Node>| Node {
            ns: ns.into(),
            local: local.into(),
            text,
            children,
            attrs: BTreeMap::new(),
            ignorable: Default::default(),
        };
        n.children = vec![leaf(
            C,
            "rich",
            String::new(),
            vec![
                leaf(A, "bodyPr", String::new(), vec![]),
                leaf(A, "lstStyle", String::new(), vec![]),
                leaf(
                    A,
                    "p",
                    String::new(),
                    vec![leaf(
                        A,
                        "r",
                        String::new(),
                        vec![leaf(A, "t", value, vec![])],
                    )],
                ),
            ],
        )];
    }
    Ok(())
}
/// Resolve visible branches before applying binary/embedding refusal. Only parts
/// reached solely through deleted dependencies may be collected.
pub(super) fn prepare(parts: &mut BTreeMap<String, Vec<u8>>, report: &mut Report) -> Result<()> {
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
            chart(&mut n, part, report)?;
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
    // Pruned declarations cannot survive into the generic dangling-type check.
    if let Some(ct) = trees.get_mut("[Content_Types].xml") {
        ct.children.retain(|n| {
            n.local != "Override"
                || n.attr("PartName")
                    .is_some_and(|p| parts.contains_key(p.trim_start_matches('/')))
        });
    }
    for (part, mut n) in trees.into_iter().chain(rels) {
        cleanse(&mut n, &part, report)?;
        parts.insert(part, serialize(&n)?);
    }
    Ok(())
}
