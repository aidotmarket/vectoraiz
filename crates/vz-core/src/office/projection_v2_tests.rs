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
    for kind in ["html", "htm"] {
        assert!(extract(
            kind,
            include_bytes!("../../../../spike/fixtures/documents/boats.pdf")
        )
        .err()
        .unwrap()
        .to_string()
        .contains("CONTAINER_MISMATCH"));
    }
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
            // Public PDFs now reach the actual decoded-stream policy instead
            // of being refused for their PDF 1.5 structure.
            let error = extract_with_pdfium("pdf", bytes, Some(&library()))
                .err()
                .unwrap();
            let reason = if name == "en-nasa-0003.pdf" {
                "PROJECTION_EXPANSION_RATIO_EXCEEDED"
            } else {
                "PROJECTION_PDF_STREAM_UNSUPPORTED"
            };
            assert!(error.to_string().contains(reason), "{error:#}");
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

#[test]
fn projection_pdf_metadata_corpus() {
    let source = std::path::Path::new(file!());
    let root = if source.is_absolute() {
        source.parent().unwrap().join("../../../..")
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    };
    let mut paths = Vec::new();
    for dir in [
        root.join("spike/fixtures/documents"),
        root.join("crates/vz-core/tests/fixtures"),
    ] {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|s| s == "pdf") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    let mut counts = BTreeMap::new();
    for path in &paths {
        let bytes = std::fs::read(path).unwrap();
        let result = super::pdf::metadata(&bytes, &mut Projection::new());
        let reason = match result {
            Ok(_) => "PASS".to_string(),
            Err(e) => e.to_string(),
        };
        println!("{}: {reason}", path.file_name().unwrap().to_string_lossy());
        *counts
            .entry(
                reason
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .trim_end_matches(':')
                    .to_string(),
            )
            .or_insert(0) += 1;
    }
    println!("PDF metadata corpus: {} files; {counts:?}", paths.len());
    assert_eq!(paths.len(), 6);
    assert_eq!(counts.get("PASS"), Some(&4));
    assert_eq!(counts.get("PROJECTION_EXPANSION_RATIO_EXCEEDED"), Some(&1));
    assert_eq!(counts.get("PROJECTION_PDF_STREAM_UNSUPPORTED"), Some(&1));
}

#[test]
fn projection_pdf_object_and_xref_streams_exclude_carriers() {
    let mut doc = Document::load_mem(&pdf(
        &["BT /F1 12 Tf 30 750 Td (MODERN_VISIBLE) Tj ET"],
        true,
    ))
    .unwrap();
    let mut bytes = Vec::new();
    doc.save_modern(&mut bytes).unwrap();
    let parsed = Document::load_mem(&bytes).unwrap();
    assert!(parsed
        .reference_table
        .entries
        .values()
        .any(|entry| matches!(entry, lopdf::xref::XrefEntry::Compressed { .. })));
    assert!(matches!(
        parsed.reference_table.cross_reference_type,
        lopdf::xref::XrefType::CrossReferenceStream
    ));
    let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
    assert!(values(&p).contains("MODERN_VISIBLE"));
    assert!(!values(&p).contains("CANARY"));
    for category in [
        "properties",
        "annotations",
        "attachments",
        "actions",
        "hidden-form-data",
    ] {
        assert!(p.drops[category] > 0, "{category}");
    }
    // The catalog is compressed: byte scanning alone cannot prove the absence
    // of optional content. Check decoded dictionary names as well.
    doc.catalog_mut()
        .unwrap()
        .set("OCProperties", dictionary! {});
    bytes.clear();
    doc.save_modern(&mut bytes).unwrap();
    assert!(super::pdf::metadata(&bytes, &mut Projection::new())
        .unwrap_err()
        .to_string()
        .contains("OPTIONAL_CONTENT_STATE_UNPROVEN"));
}

#[test]
fn projection_pdf_incremental_final_text_excludes_all_revision_carriers() {
    let original = pdf(&["BT /F1 12 Tf 30 750 Td (OLD_PAGE_CANARY) Tj ET"], true);
    let doc = Document::load_mem(&original).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let catalog = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let mut revision = lopdf::IncrementalDocument::create_from(original, doc);
    revision.opt_clone_object_to_new_document(page).unwrap();
    let content = revision.new_document.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 12 Tf 30 750 Td (FINAL_VISIBLE) Tj ET".to_vec(),
    ));
    revision
        .new_document
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content);
    let info = revision
        .new_document
        .add_object(dictionary! {"Author" => Object::string_literal("NEW_INFO_CANARY")});
    revision.new_document.trailer.set("Info", info);
    let mut bytes = Vec::new();
    revision.save_to(&mut bytes).unwrap();
    for remove_final_carriers in [false, true] {
        if remove_final_carriers {
            let doc = Document::load_mem(&bytes).unwrap();
            let mut revision = lopdf::IncrementalDocument::create_from(bytes, doc);
            revision.opt_clone_object_to_new_document(page).unwrap();
            let page = revision
                .new_document
                .get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap();
            page.remove(b"Annots");
            page.remove(b"AA");
            revision.opt_clone_object_to_new_document(catalog).unwrap();
            let catalog = revision
                .new_document
                .get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap();
            for key in [b"Metadata".as_slice(), b"AcroForm", b"Names"] {
                catalog.remove(key);
            }
            bytes = Vec::new();
            revision.save_to(&mut bytes).unwrap();
        }
        assert!(bytes.windows(5).any(|s| s == b"/Prev"));
        let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
        assert!(values(&p).contains("FINAL_VISIBLE"));
        assert!(!values(&p).contains("CANARY"), "{}", values(&p));
    }
    // A final trailer pointing outside the input cannot establish a revision.
    let offset = bytes.len();
    bytes.extend_from_slice(format!("\nxref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 1 /Root 1 0 R /Prev 99999999 >>\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    assert!(super::pdf::metadata(&bytes, &mut Projection::new()).is_err());
}

#[test]
fn projection_pdf_object_stream_decode_limits_are_not_bypassed() {
    for (filter, content, reason) in [
        ("UnsupportedFilter", vec![0; 100], "STREAM_UNSUPPORTED"),
        (
            "FlateDecode",
            {
                use std::io::Write;
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
                encoder.write_all(&vec![b' '; 100_000]).unwrap();
                encoder.finish().unwrap()
            },
            "EXPANSION_RATIO_EXCEEDED",
        ),
    ] {
        // Write the stream explicitly: lopdf's classic writer deliberately
        // omits existing ObjStm objects when rewriting a document.
        let mut bytes = pdf(&["BT /F1 12 Tf 30 750 Td (TEXT) Tj ET"], false);
        let doc = Document::load_mem(&bytes).unwrap();
        let id = doc.max_id + 1;
        let object_offset = bytes.len();
        bytes.extend_from_slice(format!("\n{id} 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Filter /{filter} /Length {} >>\nstream\n", content.len()).as_bytes());
        bytes.extend_from_slice(&content);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let xref_offset = bytes.len();
        bytes.extend_from_slice(format!("xref\n{id} 1\n{object_offset:010} 00000 n \ntrailer\n<< /Size {} /Root 1 0 R /Prev {} >>\nstartxref\n{xref_offset}\n%%EOF\n", id + 1, doc.xref_start).as_bytes());
        let error = super::pdf::metadata(&bytes, &mut Projection::new()).unwrap_err();
        assert!(error.to_string().contains(reason), "{error:#}");
    }
}
