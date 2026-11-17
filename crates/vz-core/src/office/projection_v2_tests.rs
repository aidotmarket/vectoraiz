//! CC0 local fixtures and bounded profile regression tests.
use super::*;
use lopdf::{dictionary, Document, Object, Stream};

fn library() -> std::path::PathBuf {
    std::env::var_os("VZ_TEST_PDFIUM")
        .map(Into::into)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.assets")
                .join(if cfg!(windows) {
                    "pdfium/bin/pdfium.dll"
                } else if cfg!(target_os = "macos") {
                    "pdfium/lib/libpdfium.dylib"
                } else {
                    "pdfium/lib/libpdfium.so"
                })
        })
}
fn pdf(contents: &[&str], carriers: bool) -> Vec<u8> {
    let mut doc = Document::load_mem(include_bytes!(
        "../../../../spike/fixtures/documents/boats.pdf"
    ))
    .unwrap();
    let mut kids = Vec::new();
    let base = doc.get_object((3, 0)).unwrap().as_dict().unwrap().clone();
    for content in contents {
        let stream = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let mut page = base.clone();
        page.set("Contents", stream);
        if carriers {
            page.set("Annots", vec![Object::Dictionary(dictionary! {"Subtype" => "Text", "Contents" => Object::string_literal("ANNOTATION_CANARY")})]);
            page.set("AA", dictionary! {"O" => dictionary! {"S" => "JavaScript", "JS" => Object::string_literal("ACTION_CANARY")}});
        }
        kids.push(Object::Reference(doc.add_object(page)));
    }
    let pages = doc.get_object_mut((2, 0)).unwrap().as_dict_mut().unwrap();
    pages.set("Count", kids.len() as i64);
    pages.set("Kids", kids);
    if carriers {
        let info = doc.add_object(dictionary! {"Author" => Object::string_literal("INFO_CANARY")});
        doc.trailer.set("Info", info);
        let metadata = doc.add_object(Stream::new(
            dictionary! {"Type" => "Metadata", "Subtype" => "XML"},
            b"XMP_CANARY".to_vec(),
        ));
        let catalog = doc.get_object_mut((1, 0)).unwrap().as_dict_mut().unwrap();
        catalog.set("Metadata", metadata);
        catalog.set("AcroForm", dictionary! {"Fields" => vec![Object::Dictionary(dictionary! {"V" => Object::string_literal("FORM_CANARY")})]});
        catalog.set("Names", dictionary! {"EmbeddedFiles" => dictionary! {"Names" => vec![Object::string_literal("ATTACHMENT_CANARY")]}});
    }
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}
fn values(p: &Projection) -> String {
    p.nodes
        .iter()
        .map(|n| n.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn projection_exclusion_canaries_pdf_and_geometry() {
    let bytes = pdf(&["BT /F1 12 Tf 30 750 Td (VISIBLE) Tj ET BT /F1 12 Tf 3 Tr 30 700 Td (INVISIBLE) Tj ET BT /F1 12 Tf 0 Tr 1500 700 Td (OFFPAGE) Tj ET", ""], true);
    let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
    assert!(!values(&p).contains("CANARY"));
    for value in ["VISIBLE", "INVISIBLE", "OFFPAGE"] {
        assert!(values(&p).contains(value), "{value}");
    }
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("INVISIBLE") && n.kind.contains("render mode 3") && !n.hidden));
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("OFFPAGE") && n.kind.contains("off-page") && !n.hidden));
    for category in [
        "properties",
        "annotations",
        "attachments",
        "actions",
        "hidden-form-data",
    ] {
        assert!(p.drops[category] > 0, "{category}");
    }
    assert_eq!(p.drops["no-text-pages"], 1);
    assert!(p
        .omissions
        .iter()
        .any(|(at, reason)| at == "pdf/page[2]" && reason == "no-text-pages"));
}

#[test]
fn projection_format_limits_pdf_refusals_and_public_fixture() {
    let p = extract_with_pdfium(
        "pdf",
        include_bytes!("../../../../spike/fixtures/documents/boats.pdf"),
        Some(&library()),
    )
    .unwrap();
    assert!(values(&p).contains("Harbour"));
    let error = extract_with_pdfium("pdf", &pdf(&["", ""], false), Some(&library()))
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("needs OCR; no sale projection available"),
        "{error:#}"
    );
    for (name, reason) in [
        ("OCProperties", "OPTIONAL_CONTENT_STATE_UNPROVEN"),
        ("O#43Properties", "OPTIONAL_CONTENT_STATE_UNPROVEN"),
        ("Encrypt", "ENCRYPTED"),
        ("Prev", "STRUCTURE_UNSUPPORTED"),
        ("ObjStm", "STRUCTURE_UNSUPPORTED"),
    ] {
        let mut bytes = pdf(&["BT /F1 12 Tf 30 750 Td (TEXT) Tj ET"], false);
        bytes.extend_from_slice(format!("\n/{name} ").as_bytes());
        let error = extract_with_pdfium("pdf", &bytes, Some(&library()))
            .err()
            .unwrap();
        assert!(error.to_string().contains(reason), "{error:#}");
    }
    assert!(extract_with_pdfium("pdf", b"%PDF-bad", Some(&library())).is_err());
    assert!(extract(
        "pdf",
        include_bytes!("../../../../spike/fixtures/documents/boats.pdf")
    )
    .err()
    .unwrap()
    .to_string()
    .contains("PDFIUM_UNAVAILABLE"));
}

#[test]
fn projection_exclusion_canaries_html_and_hidden_seller_choice() {
    let p = extract(
        "html",
        include_bytes!("../../tests/fixtures/projection-v2.html"),
    )
    .unwrap();
    assert!(!values(&p).contains("DROP_CANARY"));
    for value in [
        "ATTR_HIDDEN",
        "DISPLAY_HIDDEN",
        "VISIBILITY_HIDDEN",
        "TEMPLATE_HIDDEN",
        "INHERITED_HIDDEN",
        "ESCAPED_HIDDEN",
    ] {
        assert!(
            p.nodes.iter().any(|n| n.text.contains(value) && n.hidden),
            "{value}"
        );
    }
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("STYLESHEET_CONTENT") && !n.hidden));
    assert!(p.warnings.iter().any(|s| s.contains("not evaluated")));
    for category in ["metadata", "comments", "active-markup", "hidden-form-data"] {
        assert!(p.drops[category] > 0, "{category}");
    }
    assert!(extract(
        "htm",
        include_bytes!("../../../../spike/fixtures/documents/gardens.html")
    )
    .is_ok());
    assert!(extract("html", b"<svg><text>unsupported</text></svg>").is_err());
    let deep = format!("{}text{}", "<div>".repeat(260), "</div>".repeat(260));
    assert!(extract("html", deep.as_bytes())
        .err()
        .unwrap()
        .to_string()
        .contains("DEPTH_EXCEEDED"));
}

#[test]
fn projection_real_public_corpus_and_recorded_licenses() {
    let sources: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/projection-v2-sources.json"
    ))
    .unwrap();
    for (name, bytes) in [
        (
            "projection-v2.html",
            include_bytes!("../../tests/fixtures/projection-v2.html").as_slice(),
        ),
        (
            "projection-v2.pdf",
            include_bytes!("../../tests/fixtures/projection-v2.pdf").as_slice(),
        ),
        (
            "en-nasa-0003.pdf",
            include_bytes!("../../tests/fixtures/en-nasa-0003.pdf").as_slice(),
        ),
        (
            "en-nasa-0010.pdf",
            include_bytes!("../../tests/fixtures/en-nasa-0010.pdf").as_slice(),
        ),
        (
            "wiki-current.html",
            include_bytes!("../../tests/fixtures/wiki-current.html").as_slice(),
        ),
    ] {
        let source = sources
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["file"] == name || s["filename"] == name)
            .unwrap();
        assert_eq!(source["sha256"], crate::pipeline::sha256(bytes));
        assert!(source["license"].as_str().is_some_and(|s| !s.is_empty()));
        if name.starts_with("en-nasa") {
            let error = extract_with_pdfium("pdf", bytes, Some(&library()))
                .err()
                .unwrap();
            assert!(
                error
                    .to_string()
                    .contains("PROJECTION_PDF_STRUCTURE_UNSUPPORTED"),
                "{error:#}"
            );
        } else if name == "wiki-current.html" {
            let p = extract("html", bytes).unwrap();
            assert!(p.nodes.len() > 1_000);
            assert!(values(&p).contains("2010"));
        }
    }
}

#[test]
fn projection_pdf_malformed_graph_and_image_only_refusals() {
    for bad in ["count", "dangling", "syntax"] {
        let mut doc =
            Document::load_mem(&pdf(&["BT /F1 12 Tf 30 700 Td (TEXT) Tj ET"], false)).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        match bad {
            "count" => doc
                .get_object_mut((2, 0))
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Count", 9),
            "dangling" => doc
                .get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Contents", (99_999, 0)),
            _ => {
                let broken = doc.add_object(Stream::new(dictionary! {}, b"BT (unclosed".to_vec()));
                doc.get_object_mut(page)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Contents", broken);
            }
        }
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let error = extract_with_pdfium("pdf", &bytes, Some(&library()))
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("MALFORMED") || error.contains("PAGE_TREE_UNPROVEN"),
            "{bad}: {error}"
        );
    }
    let mut doc = Document::load_mem(&pdf(&["q 100 0 0 100 30 30 cm /Im1 Do Q"], false)).unwrap();
    let image = doc.add_object(Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8}, vec![0]));
    let page = *doc.get_pages().values().next().unwrap();
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Resources",
            dictionary! {"XObject" => dictionary! {"Im1" => image}},
        );
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert!(extract_with_pdfium("pdf", &bytes, Some(&library()))
        .err()
        .unwrap()
        .to_string()
        .contains("PROJECTION_PDF_NEEDS_OCR"));
}
