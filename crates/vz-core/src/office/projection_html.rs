//! Inert HTML5 tree. No URL, attribute, DOM serialization or CSS layout output.
use super::*;
use scraper::{Html, Node as HtmlNode};

pub(super) fn extract(input: &[u8], out: &mut Projection) -> Result<()> {
    ensure!(
        input.len() as u64 <= PART_BYTES,
        "PROJECTION_PART_BYTES_EXCEEDED at html: {} / {PART_BYTES}",
        input.len()
    );
    let text = decode_html(input)?;
    ensure!(
        !text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("%PDF")
            && !input.starts_with(b"PK")
            && !input.starts_with(&[0xd0, 0xcf, 0x11, 0xe0]),
        "PROJECTION_HTML_CONTAINER_MISMATCH: container disguised as HTML"
    );
    ensure!(!text.contains('\0'), "PROJECTION_HTML_MALFORMED: NUL byte");
    let html = Html::parse_document(&text);
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
            .unwrap_or((false, false, 0usize, 0usize));
        let (mut hidden, mut dropped, depth, mut block) = parent;
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
                if matches!(
                    name,
                    "address"
                        | "article"
                        | "aside"
                        | "blockquote"
                        | "body"
                        | "details"
                        | "summary"
                        | "center"
                        | "dialog"
                        | "option"
                        | "div"
                        | "dl"
                        | "dt"
                        | "dd"
                        | "fieldset"
                        | "figcaption"
                        | "figure"
                        | "footer"
                        | "form"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "header"
                        | "hr"
                        | "li"
                        | "main"
                        | "nav"
                        | "ol"
                        | "p"
                        | "pre"
                        | "section"
                        | "table"
                        | "td"
                        | "th"
                        | "tr"
                        | "ul"
                ) {
                    block = index;
                }
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
                    let control =
                        matches!(name, "input" | "textarea" | "select" | "button" | "output");
                    let own_hidden =
                        element.attr("hidden").is_some() || style.is_some_and(inline_hidden);
                    let hidden_form = control
                        && (hidden
                            || own_hidden
                            || element
                                .attr("type")
                                .is_some_and(|t| t.eq_ignore_ascii_case("hidden")));
                    if control && !hidden_form {
                        out.drop_at("form-control-state", &at)?;
                    }
                    let category = match name {
                        _ if hidden_form => Some("hidden-form-data"),
                        "input" | "textarea" | "select" | "output" => {
                            dropped = true;
                            None
                        }
                        "svg" | "math" => Some("unsupported-markup"),
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
                    for (key, value) in element.attrs() {
                        if key.to_ascii_lowercase().starts_with("on")
                            || matches!(key, "srcdoc" | "action" | "formaction")
                            || (matches!(key, "href" | "src" | "xlink:href")
                                && value
                                    .trim_start()
                                    .to_ascii_lowercase()
                                    .starts_with("javascript:"))
                        {
                            out.drop_at("active-markup", &format!("{at}/@{key}"))?;
                        }
                    }
                    hidden |= element.attr("hidden").is_some()
                        || matches!(
                            name,
                            "template"
                                | "datalist"
                                | "canvas"
                                | "video"
                                | "audio"
                                | "noembed"
                                | "noframes"
                                | "rp"
                                | "progress"
                                | "meter"
                        )
                        || (name == "dialog" && element.attr("open").is_none())
                        || style.is_some_and(inline_hidden);
                }
                if name == "br" && !dropped {
                    out.add(
                        &format!("html/block[{block}]/flow[{index}]"),
                        "HTML body text",
                        "\n".into(),
                        hidden,
                    )?;
                }
            }
            HtmlNode::Text(value) if !dropped => {
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
                out.add(
                    &format!("html/block[{block}]/flow[{index}]"),
                    kind,
                    value.to_string(),
                    hidden,
                )?;
            }
            _ => (),
        }
        states.insert(node.id(), (hidden, dropped, depth + 1, block));
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

// BOM wins over metadata. Only a meta element can declare an encoding; body
// text and arbitrary attributes cannot switch the decoder.
fn decode_html(input: &[u8]) -> Result<String> {
    use encoding_rs::{Encoding, UTF_8};
    let (encoding, skip) = if let Some(bom) = Encoding::for_bom(input) {
        bom
    } else {
        let sniff: String = input.iter().take(4096).map(|b| char::from(*b)).collect();
        let tree = Html::parse_document(&sniff);
        let selector = scraper::Selector::parse("meta").unwrap();
        let mut label = None;
        for meta in tree.select(&selector) {
            if let Some(value) = meta.value().attr("charset") {
                label = Some(value.to_string());
                break;
            }
            if meta
                .value()
                .attr("http-equiv")
                .is_some_and(|s| s.eq_ignore_ascii_case("content-type"))
            {
                if let Some(content) = meta.value().attr("content") {
                    let lower = content.to_ascii_lowercase();
                    if let Some((_, tail)) = lower.split_once("charset=") {
                        label = Some(
                            tail.trim()
                                .trim_matches(['\'', '"'])
                                .split(';')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string(),
                        );
                        break;
                    }
                }
            }
        }
        let enc = match label {
            Some(label) => Encoding::for_label(label.as_bytes())
                .context("PROJECTION_HTML_ENCODING_UNSUPPORTED: declared charset")?,
            None => UTF_8,
        };
        // HTML meta UTF-16 labels mean UTF-8; only a BOM selects UTF-16.
        let enc = if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE {
            UTF_8
        } else {
            enc
        };
        (enc, 0)
    };
    let (text, errors) = encoding.decode_without_bom_handling(&input[skip..]);
    ensure!(
        !errors,
        "PROJECTION_HTML_ENCODING_UNSUPPORTED: invalid encoded bytes"
    );
    Ok(text.into_owned())
}
