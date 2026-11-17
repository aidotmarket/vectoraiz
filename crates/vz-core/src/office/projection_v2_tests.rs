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
