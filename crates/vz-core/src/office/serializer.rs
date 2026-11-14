//! Scoped namespace serialization for delivered OPC parts.
use super::*;
use quick_xml::{
    events::{BytesDecl, BytesEnd, BytesStart, BytesText},
    Writer,
};

type Bindings = BTreeMap<String, String>;

pub(super) fn validate_mc(n: &mut Node, location: &str) -> Result<()> {
    for ((ns, key), value) in &mut n.attrs {
        let requires = n.ns == MC && n.local == "Choice" && ns.is_empty() && key == "Requires";
        if !requires && !(ns == MC && words("Ignorable MustUnderstand ProcessContent", key)) {
            continue;
        }
        let mut valid = Vec::new();
        for token in value.split_ascii_whitespace() {
            let prefix = if key == "ProcessContent" {
                token.split_once(':').map(|(p, _)| p).unwrap_or("")
            } else {
                token
            };
            let resolved = !prefix.is_empty()
                && !prefix.contains(':')
                && (prefix == "xml" || n.bindings.get(prefix).is_some_and(|uri| !uri.is_empty()));
            if resolved {
                valid.push(token);
            } else if key != "Ignorable" {
                bail!("unbound MC prefix {token} at {location}/@{key}");
            }
        }
        ensure!(
            !requires || !valid.is_empty(),
            "empty mc:Choice Requires at {location}/@Requires"
        );
        *value = valid.join(" ");
    }
    Ok(())
}

fn conventional(ns: &str) -> Option<&'static str> {
    Some(match ns {
        W => "w",
        S => "x",
        P => "p",
        A => "a",
        R => "r",
        MC => "mc",
        WP => "wp",
        PIC => "pic",
        XDR => "xdr",
        X14 => "x14",
        XM => "xm",
        P14 => "p14",
        "http://schemas.openxmlformats.org/drawingml/2006/chart" => "c",
        "http://schemas.openxmlformats.org/drawingml/2006/diagram" => "dgm",
        "http://schemas.microsoft.com/office/drawing/2008/diagram" => "dsp",
        "http://schemas.microsoft.com/office/drawing/2012/chartStyle" => "cs",
        "http://schemas.openxmlformats.org/officeDocument/2006/math" => "m",
        _ => return None,
    })
}

fn qname(
    ns: &str,
    local: &str,
    attribute: bool,
    source: &Bindings,
    bindings: &mut Bindings,
) -> String {
    if ns.is_empty() {
        return local.into();
    }
    if ns == XML {
        return format!("xml:{local}");
    }
    // OPC's structural namespaces must use unprefixed elements.
    if !attribute && [CT, REL].contains(&ns) {
        bindings.insert(String::new(), ns.into());
        return local.into();
    }
    if let Some((prefix, _)) = source.iter().chain(bindings.iter()).find(|(p, uri)| {
        uri.as_str() == ns && (!attribute || !p.is_empty()) && bindings.get(*p) == Some(*uri)
    }) {
        return if prefix.is_empty() {
            local.into()
        } else {
            format!("{prefix}:{local}")
        };
    }
    // SpreadsheetML conventionally uses the default namespace. Attributes
    // still need a nonempty prefix because defaults apply only to elements.
    if !attribute && ns == S && !bindings.contains_key("") {
        bindings.insert(String::new(), ns.into());
        return local.into();
    }
    let prefix = conventional(ns)
        .filter(|p| !bindings.contains_key(*p))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            (0..)
                .map(|i| format!("n{i}"))
                .find(|p| !bindings.contains_key(p))
                .unwrap()
        });
    bindings.insert(prefix.clone(), ns.into());
    format!("{prefix}:{local}")
}

pub(super) fn serialize(n: &Node) -> Result<Vec<u8>> {
    fn write(n: &Node, parent: &Bindings, w: &mut Writer<Vec<u8>>) -> Result<()> {
        let mut bindings = parent.clone();
        bindings.extend(n.bindings.iter().map(|(p, uri)| (p.clone(), uri.clone())));
        if n.ns.is_empty() {
            bindings.insert(String::new(), String::new());
        }
        let name = qname(&n.ns, &n.local, false, &n.bindings, &mut bindings);
        let mut attrs = Vec::new();
        for ((ns, key), value) in &n.attrs {
            let value = if n.ns == MC && n.local == "Choice" && ns.is_empty() && key == "Requires" {
                // The parser stores URIs here for AlternateContent selection.
                value
                    .split_ascii_whitespace()
                    .map(|uri| {
                        qname(uri, "_", true, &n.bindings, &mut bindings)
                            .trim_end_matches(":_")
                            .to_owned()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                value.clone()
            };
            attrs.push((qname(ns, key, true, &n.bindings, &mut bindings), value));
        }
        for (prefix, uri) in &bindings {
            if prefix == "xml"
                || parent.get(prefix) == Some(uri)
                || (prefix.is_empty() && uri.is_empty() && !parent.contains_key(prefix))
            {
                continue;
            }
            attrs.push((
                if prefix.is_empty() {
                    "xmlns".into()
                } else {
                    format!("xmlns:{prefix}")
                },
                uri.clone(),
            ));
        }
        let mut start = BytesStart::new(&name);
        for (key, value) in &attrs {
            let escaped = quick_xml::escape::escape(value)
                .replace('\n', "&#10;")
                .replace('\r', "&#13;")
                .replace('\t', "&#9;");
            start.push_attribute(quick_xml::events::attributes::Attribute {
                key: quick_xml::name::QName(key.as_bytes()),
                value: std::borrow::Cow::Owned(escaped.into_bytes()),
            });
        }
        w.write_event(Event::Start(start))?;
        if !n.text.is_empty() {
            w.write_event(Event::Text(BytesText::new(&n.text)))?;
        }
        for child in &n.children {
            write(child, &bindings, w)?;
        }
        w.write_event(Event::End(BytesEnd::new(&name)))?;
        Ok(())
    }
    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new(
        "1.0",
        Some("UTF-8"),
        Some("yes"),
    )))?;
    write(n, &Bindings::new(), &mut writer)?;
    Ok(writer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
    fn roundtrip(xml: &str) -> (String, Node) {
        let n = parse(xml.as_bytes(), "fixture.xml").unwrap();
        let out = String::from_utf8(serialize(&n).unwrap()).unwrap();
        assert!(out.starts_with(DECL), "{out}");
        let reopened = parse(out.as_bytes(), "delivered.xml").unwrap();
        (out, reopened)
    }
    #[test]
    fn opc_parts_use_default_namespace_and_declaration() {
        for (ns, root, child) in [
            (CT, "Types", "Override"),
            (REL, "Relationships", "Relationship"),
        ] {
            let (out, n) = roundtrip(&format!(
                "<src:{root} xmlns:src=\"{ns}\"><src:{child}/></src:{root}>"
            ));
            assert!(out.starts_with(&format!("{DECL}<{root} ")));
            assert!(out.contains(&format!("xmlns=\"{ns}\"")));
            assert!(out.contains(&format!("<{child}>")));
            assert_eq!(n.ns, ns);
            assert_eq!(n.children[0].ns, ns);
        }
    }
    #[test]
    fn source_prefixes_scopes_aliases_and_mc_values_survive() {
        let ext = "http://schemas.microsoft.com/office/word/2010/wordml";
        let (out, n) = roundtrip(&format!(
            r#"<custom:document xmlns:custom="{W}" xmlns:compat="{MC}" xmlns:feature="{ext}" compat:Ignorable="feature missing" compat:MustUnderstand="feature" compat:ProcessContent="feature:item"><custom:body xmlns:feature="urn:scoped"><feature:item/></custom:body><compat:Choice Requires="feature"><feature:item/></compat:Choice></custom:document>"#
        ));
        assert!(out.contains("<custom:document"));
        assert!(out.contains("compat:Ignorable=\"feature\""));
        assert!(out.contains("Requires=\"feature\""));
        assert_eq!(n.attr_ns(MC, "MustUnderstand"), Some("feature"));
        assert_eq!(n.attr_ns(MC, "ProcessContent"), Some("feature:item"));
        assert_eq!(n.children[0].children[0].ns, "urn:scoped");
        assert_eq!(n.children[1].attr("Requires"), Some(ext));
        assert_eq!(n.children[1].bindings["feature"], ext);
    }
    #[test]
    fn conventional_then_generated_prefixes_for_synthesized_nodes() {
        for (ns, prefix) in [
            (W, "w"),
            (A, "a"),
            (P, "p"),
            (R, "r"),
            (MC, "mc"),
            (WP, "wp"),
            (PIC, "pic"),
            (
                "http://schemas.openxmlformats.org/drawingml/2006/chart",
                "c",
            ),
            (
                "http://schemas.openxmlformats.org/drawingml/2006/diagram",
                "dgm",
            ),
            (
                "http://schemas.microsoft.com/office/drawing/2008/diagram",
                "dsp",
            ),
            (
                "http://schemas.microsoft.com/office/drawing/2012/chartStyle",
                "cs",
            ),
            ("urn:unknown", "n0"),
        ] {
            let mut n = parse(
                format!("<source:item xmlns:source=\"{ns}\"/>").as_bytes(),
                "fixture.xml",
            )
            .unwrap();
            n.bindings = Default::default();
            let out = String::from_utf8(serialize(&n).unwrap()).unwrap();
            assert!(out.contains(&format!("<{prefix}:item")), "{out}");
            assert_eq!(parse(out.as_bytes(), "out").unwrap().ns, ns);
        }
    }
    #[test]
    fn default_namespaces_do_not_qualify_attributes_or_unqualified_children() {
        let (out, n) = roundtrip(&format!(
            r#"<sheet xmlns="{S}" xmlns:x="{S}" x:qualified="yes" plain="ok" xml:space="preserve"><bare xmlns=""/></sheet>"#
        ));
        assert!(out.contains("x:qualified=\"yes\""));
        assert_eq!(n.attr_ns(S, "qualified"), Some("yes"));
        assert_eq!(n.attr("plain"), Some("ok"));
        assert_eq!(n.attr_ns(XML, "space"), Some("preserve"));
        assert!(n.children[0].ns.is_empty());
    }
    #[test]
    fn unresolved_required_mc_prefixes_refuse_at_location() {
        for attr in ["Requires", "MustUnderstand", "ProcessContent"] {
            let (el, key, value) = if attr == "Requires" {
                ("mc:Choice", attr.to_owned(), "missing")
            } else {
                (
                    "root",
                    format!("mc:{attr}"),
                    if attr == "ProcessContent" {
                        "missing:item"
                    } else {
                        "missing"
                    },
                )
            };
            let xml = format!(r#"<root xmlns:mc="{MC}"><{el} {key}="{value}"/></root>"#);
            let err = parse(xml.as_bytes(), "fixture.xml")
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(&format!("/@{attr}")) && err.contains("fixture.xml/{}root[1]/"),
                "{err}"
            );
        }
        let (_, n) = roundtrip(&format!(
            r#"<root xmlns:mc="{MC}" mc:Ignorable="missing"/>"#
        ));
        assert_eq!(n.attr_ns(MC, "Ignorable"), Some(""));
    }
    #[test]
    fn source_binding_precedes_inherited_fallback() {
        let mut n = parse(
            format!(r#"<z:document xmlns:z="{W}"><z:body/></z:document>"#).as_bytes(),
            "fixture",
        )
        .unwrap();
        n.bindings = Default::default();
        let out = String::from_utf8(serialize(&n).unwrap()).unwrap();
        assert!(
            out.contains("<w:document") && out.contains("<z:body"),
            "{out}"
        );
        parse(out.as_bytes(), "out").unwrap();
    }
    #[test]
    fn synthesized_spreadsheet_uses_default_namespace() {
        let mut n = parse(
            format!(r#"<source:worksheet xmlns:source="{S}"/>"#).as_bytes(),
            "fixture",
        )
        .unwrap();
        n.bindings = Default::default();
        let out = String::from_utf8(serialize(&n).unwrap()).unwrap();
        assert!(
            out.contains(&format!(r#"<worksheet xmlns="{S}">"#)),
            "{out}"
        );
        assert_eq!(parse(out.as_bytes(), "out").unwrap().ns, S);
    }
    #[test]
    fn generated_prefixes_avoid_source_collisions() {
        let mut n = parse(
            format!(r#"<root xmlns:w="urn:other" xmlns:n0="urn:zero"/>"#).as_bytes(),
            "fixture",
        )
        .unwrap();
        n.ns = W.into();
        let out = String::from_utf8(serialize(&n).unwrap()).unwrap();
        assert!(out.contains("<n1:root"), "{out}");
        assert_eq!(parse(out.as_bytes(), "out").unwrap().ns, W);
    }
}
