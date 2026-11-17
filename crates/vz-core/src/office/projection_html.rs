//! Inert HTML5 tree. No URL, attribute, DOM serialization or CSS layout output.
use super::*;
use scraper::{Html, Node as HtmlNode};

pub(super) fn extract(input: &[u8], out: &mut Projection) -> Result<()> {
    ensure!(
        input.len() as u64 <= PART_BYTES,
        "PROJECTION_PART_BYTES_EXCEEDED at html: {} / {PART_BYTES}",
        input.len()
    );
    let text = std::str::from_utf8(input)
        .context("PROJECTION_HTML_ENCODING_UNSUPPORTED: save UTF-8 or exclude document")?;
    ensure!(!text.contains('\0'), "PROJECTION_HTML_MALFORMED: NUL byte");
    let html = Html::parse_document(text);
    out.parts = 1;
    out.expanded_bytes = input.len() as u64;
    let mut states = BTreeMap::new();
    let mut styled = false;
    for (index, node) in html.tree.root().descendants().enumerate() {
        out.limit()?;
        ensure!(
            index < MAX_VALUES,
            "PROJECTION_INVENTORY_LIMIT_EXCEEDED at html: {index} / {MAX_VALUES}"
        );
        let parent = node
            .parent()
            .and_then(|p| states.get(&p.id()))
            .copied()
            .unwrap_or((false, false, 0usize));
        let (mut hidden, mut dropped, depth) = parent;
        ensure!(
            depth < 256,
            "PROJECTION_HTML_DEPTH_EXCEEDED at html/node[{index}]: {depth} / 256"
        );
        let at = format!("html/node[{index}]");
        match node.value() {
            HtmlNode::Comment(_) if !dropped => {
                out.drop_at("comments", &at)?;
            }
            HtmlNode::Element(element) => {
                let name = element.name();
                let style = element.attr("style");
                styled |= style.is_some()
                    || name == "style"
                    || (name == "link"
                        && element.attr("rel").is_some_and(|v| {
                            v.split_whitespace()
                                .any(|r| r.eq_ignore_ascii_case("stylesheet"))
                        }));
                if !dropped {
                    // Form control state is never a supported content label.
                    let category = match name {
                        "input" | "textarea" | "select" | "button" | "output" => {
                            Some("hidden-form-data")
                        }
                        "script" | "noscript" | "iframe" | "object" | "embed" | "applet" => {
                            Some("active-markup")
                        }
                        "head" | "meta" | "title" | "base" | "link" | "style" => Some("metadata"),
                        _ => None,
                    };
                    if let Some(category) = category {
                        // Head children are still visited for individual carrier counts.
                        if name != "head" {
                            out.drop_at(category, &at)?;
                            dropped = true;
                        }
                    }
                    for (key, _) in element.attrs() {
                        if key.to_ascii_lowercase().starts_with("on")
                            || matches!(key, "srcdoc" | "action" | "formaction")
                        {
                            out.drop_at("active-markup", &format!("{at}/@{key}"))?;
                        }
                    }
                    ensure!(
                        !matches!(name, "svg" | "math"),
                        "PROJECTION_HTML_CONTENT_UNSUPPORTED at {at}: {name}"
                    );
                    hidden |= element.attr("hidden").is_some()
                        || name == "template"
                        || style.is_some_and(inline_hidden);
                }
            }
            HtmlNode::Text(value) if !dropped && !value.trim().is_empty() => {
                let parent_name = node
                    .parent()
                    .and_then(|n| n.value().as_element().map(|e| e.name().to_string()))
                    .unwrap_or_default();
                let kind = match parent_name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "HTML heading",
                    "td" | "th" => "HTML table text",
                    "label" | "legend" | "caption" | "figcaption" => "HTML label",
                    _ => "HTML body text",
                };
                out.add(&at, kind, value.to_string(), hidden)?;
            }
            _ => (),
        }
        states.insert(node.id(), (hidden, dropped, depth + 1));
    }
    if styled {
        out.warnings.push("HTML visibility limitation: stylesheets, classes and CSS layout are not evaluated; only hidden, inline display:none, inline visibility:hidden and template contents are classified (inherited). No resources are fetched.".into());
    }
    Ok(())
}

fn inline_hidden(style: &str) -> bool {
    use cssparser::{Delimiter, Parser, ParserInput};
    let mut input = ParserInput::new(style);
    let mut parser = Parser::new(&mut input);
    let mut hidden = false;
    while !parser.is_exhausted() {
        let result: std::result::Result<bool, cssparser::ParseError<'_, ()>> = parser
            .parse_until_before(Delimiter::Semicolon, |p| {
                let name = p.expect_ident_cloned()?;
                p.expect_colon()?;
                let value = p.expect_ident_cloned()?;
                if !p.is_exhausted() {
                    p.expect_delim('!')?;
                    p.expect_ident_matching("important")?;
                }
                p.expect_exhausted()?;
                Ok(
                    (name.eq_ignore_ascii_case("display") && value.eq_ignore_ascii_case("none"))
                        || (name.eq_ignore_ascii_case("visibility")
                            && value.eq_ignore_ascii_case("hidden")),
                )
            });
        hidden |= result.unwrap_or(false);
        let _ = parser.expect_semicolon();
    }
    hidden
}
