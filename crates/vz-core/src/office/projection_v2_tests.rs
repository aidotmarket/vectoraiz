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
        ("O#43", "OPTIONAL_CONTENT_STATE_UNPROVEN"),
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
    assert_eq!(
        extract("html", b"<p>visible</p><svg><text>unsupported</text></svg>")
            .unwrap()
            .drops["unsupported-markup"],
        1
    );
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
            let p = extract_with_pdfium("pdf", bytes, Some(&library())).unwrap();
            assert!(!values(&p).trim().is_empty());
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
            error.contains("MALFORMED")
                || error.contains("PAGE_TREE_UNPROVEN")
                || (bad == "dangling" && error.contains("NEEDS_OCR")),
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
    assert_eq!(counts.get("PASS"), Some(&6));
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
                encoder.write_all(&vec![b' '; 5_000_000]).unwrap();
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

#[test]
fn projection_pdf_xref_stream_expansion_is_bounded_before_loading() {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&vec![0; 100_000]).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut bytes = b"%PDF-1.5\n".to_vec();
    let offset = bytes.len();
    bytes.extend_from_slice(format!("1 0 obj\n<< /Type /XRef /Filter /FlateDecode /W [1 4 2] /Size 2 /Length {} >>\nstream\n", compressed.len()).as_bytes());
    bytes.extend_from_slice(&compressed);
    bytes
        .extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    let error = super::pdf::metadata(&bytes, &mut Projection::new()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("PROJECTION_EXPANSION_RATIO_EXCEEDED"),
        "{error:#}"
    );
}

fn save_pdf(mut doc: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn projection_pdf_nested_forms_validate_before_text_loading() {
    for malformed in [false, true] {
        let mut doc = Document::load_mem(&pdf(
            &["BT /F1 12 Tf 30 750 Td (MAIN_VISIBLE) Tj ET /Outer Do"],
            false,
        ))
        .unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let resources = super::pdf::test_resources(&doc, page);
        let inner = doc.add_object(Stream::new(dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(),0.into(),600.into(),800.into()], "Resources" => resources,
        }, if malformed {b"BT /F1 12 Tf 30 650 Td (UNCLOSED".to_vec()} else {b"BT /F1 12 Tf 30 650 Td (FORM_VISIBLE) Tj ET".to_vec()}));
        let outer = doc.add_object(Stream::new(dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(),0.into(),600.into(),800.into()],
            "Resources" => dictionary! {"XObject" => dictionary! {"Inner" => inner}},
        }, b"/Inner Do".to_vec()));
        doc.get_object_mut(page).unwrap().as_dict_mut().unwrap().set("Resources", dictionary! {"Font" => super::pdf::test_resources(&Document::load_mem(&pdf(&[""], false)).unwrap(), (3,0)).get(b"Font").unwrap().clone(), "XObject" => dictionary! {"Outer" => outer}});
        let bytes = save_pdf(doc);
        if malformed {
            // No library is supplied: the named refusal must precede PDFium,
            // and therefore precedes all downstream embedding work.
            let error = extract_with_pdfium("pdf", &bytes, None).err().unwrap();
            assert!(
                format!("{error:#}").contains("PROJECTION_PDF_MALFORMED"),
                "{error:#}"
            );
        } else {
            let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
            assert!(values(&p).contains("MAIN_VISIBLE") && values(&p).contains("FORM_VISIBLE"));
        }
    }
}

#[test]
fn projection_pdf_inherited_boxes_null_refs_actualtext_and_annotation_counts() {
    let mut doc = Document::load_mem(&pdf(&["BT /F1 12 Tf 30 750 Td (VISIBLE) Tj ET BT /F1 12 Tf 400 600 Td (OFFPAGE) Tj ET /Span << /ActualText (REPLACEMENT) >> BDC BT /F1 12 Tf 30 650 Td (GLYPHS) Tj ET EMC"], false)).unwrap();
    for id in doc.get_pages().values().copied().collect::<Vec<_>>() {
        let page = doc.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        page.remove(b"MediaBox");
        page.remove(b"CropBox");
        page.set("Unused", (99_999, 0));
        page.set(
            "Annots",
            vec![
                Object::Dictionary(dictionary! {"Subtype"=>"Text"}),
                Object::Dictionary(dictionary! {"Subtype"=>"Text"}),
                Object::Reference((99_999, 0)),
            ],
        );
    }
    let pages = doc.get_object_mut((2, 0)).unwrap().as_dict_mut().unwrap();
    pages.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
    pages.set("CropBox", vec![0.into(), 0.into(), 300.into(), 792.into()]);
    doc.catalog_mut()
        .unwrap()
        .set("VZGuardFailure", Object::string_literal("FORGED_REASON"));
    // A structure attribute dictionary is not an action. A link action is.
    doc.catalog_mut().unwrap().set("StructTreeRoot", dictionary! {"K"=>dictionary!{"Type"=>"StructElem","S"=>"P","A"=>dictionary!{"O"=>"Layout"}}});
    let page = *doc.get_pages().values().next().unwrap();
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "A",
            dictionary! {"S"=>"URI","URI"=>Object::string_literal("https://example.invalid/")},
        );
    let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
    assert_eq!(p.drops["annotations"], 2);
    assert_eq!(p.drops["actions"], 1);
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("OFFPAGE") && n.kind.contains("off-page")));
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("REPLACEMENT") && n.kind.contains("not-visible ActualText")));
}

fn image_pdf(contents: &[&str], filter: &str, data: Vec<u8>) -> Vec<u8> {
    let mut doc = Document::load_mem(&pdf(contents, false)).unwrap();
    let image = doc.add_object(Stream::new(dictionary! {"Type"=>"XObject", "Subtype"=>"Image", "Width"=>400, "Height"=>400, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8, "Filter"=>filter}, data));
    for id in doc.get_pages().values().copied().collect::<Vec<_>>() {
        let mut resources = super::pdf::test_resources(&doc, id);
        resources.set("XObject", dictionary! {"Im1"=>image});
        doc.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", resources);
    }
    save_pdf(doc)
}

#[test]
fn projection_pdf_image_decoders_are_not_needed_for_text_loading() {
    // Each invoked image has a deliberately invalid encoded payload. A text
    // projection must succeed independently of image decoder success. Includes
    // codecs absent from lopdf and a Flate payload with no valid zlib stream.
    for filter in [
        "DCTDecode",
        "CCITTFaxDecode",
        "JBIG2Decode",
        "JPXDecode",
        "FlateDecode",
        "UnsupportedImageCodec",
    ] {
        let bytes = image_pdf(
            &[
                "BT /F1 12 Tf 30 750 Td (VISIBLE) Tj ET q 400 0 0 400 0 0 cm /Im1 Do Q",
                "q /Im1 Do Q",
            ],
            filter,
            vec![0x78, 0x9c, 0xff, 0xff],
        );
        let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
        assert!(values(&p).contains("VISIBLE"), "{filter}");
        assert_eq!(p.drops["no-text-pages"], 1, "{filter}");
        let bytes = image_pdf(&["/Im1 Do"], filter, vec![0x78, 0x9c, 0xff, 0xff]);
        let error = extract_with_pdfium("pdf", &bytes, Some(&library()))
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("PROJECTION_PDF_NEEDS_OCR"),
            "{filter}: {error:#}"
        );
    }
    let bytes = image_pdf(
        &["/Im1 Do BT /F1 12 Tf 3 Tr 30 700 Td (OCR_LAYER) Tj ET"],
        "DCTDecode",
        vec![0; 16],
    );
    let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("OCR_LAYER") && n.kind.contains("invisible")));
}

#[test]
fn projection_pdf_single_filter_png_predictor_and_false_stream_word() {
    use std::io::Write;
    let content = b"BT /F1 12 Tf 30 700 Td (FILTER_ARRAY) Tj ET";
    let mut doc = Document::load_mem(&pdf(&[""], false)).unwrap();
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&[vec![0], content.to_vec()].concat())
        .unwrap();
    let stream=doc.add_object(Stream::new(dictionary! {"Filter"=>vec![Object::Name(b"FlateDecode".to_vec())], "DecodeParms"=>dictionary! {"Predictor"=>12,"Columns"=>content.len() as i64}},encoder.finish().unwrap()));
    let id = *doc.get_pages().values().next().unwrap();
    doc.get_object_mut(id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", stream);
    let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
    assert!(values(&p).contains("FILTER_ARRAY"));
    let mut bytes = pdf(
        &["BT /F1 12 Tf 30 700 Td (stream\n\\170\\234 fake payload) Tj ET"],
        false,
    );
    bytes.extend_from_slice(b"\n% stream\nx\x9c invalid zlib-like bytes\n");
    assert!(extract_with_pdfium("pdf", &bytes, Some(&library())).is_ok());
}

#[test]
fn projection_html_controls_hidden_fallback_charsets_and_flow() {
    let p=extract("html",br#"<p>Hello <b>bold</b>world</p><p>Next block</p><label>Label<input value="DROP_CANARY"></label><textarea>DROP_CANARY</textarea><select><option>DROP_CANARY</option></select><output>DROP_CANARY</output><button value="DROP_CANARY">Button text</button><svg><text>DROP_CANARY</text></svg><div hidden><math>DROP_CANARY</math></div><datalist>UA_HIDDEN</datalist><dialog>UA_HIDDEN</dialog><canvas>UA_HIDDEN</canvas><video>UA_HIDDEN</video><audio>UA_HIDDEN</audio><noembed>UA_HIDDEN</noembed><noframes>UA_HIDDEN</noframes><ruby>x<rp>UA_HIDDEN</rp></ruby><dialog open>OPEN_DIALOG</dialog>"#).unwrap();
    assert!(!values(&p).contains("DROP_CANARY"));
    assert!(values(&p).contains("Button text"));
    assert_eq!(p.drops["form-control-state"], 5);
    assert_eq!(p.drops["unsupported-markup"], 2);
    assert_eq!(
        p.nodes
            .iter()
            .filter(|n| n.text.contains("UA_HIDDEN") && n.hidden)
            .count(),
        8
    );
    let nodes: Vec<_> = p
        .nodes
        .iter()
        .filter(|n| ["Hello ", "bold", "world"].contains(&n.text.as_str()))
        .collect();
    assert_eq!(nodes.len(), 3);
    assert!(nodes
        .iter()
        .all(|n| n.location.rsplit_once("/flow[").unwrap().0
            == nodes[0].location.rsplit_once("/flow[").unwrap().0));
    for (bytes, expected) in [
        (
            b"<meta charset=windows-1252><p>caf\xe9 \x80</p>".as_slice(),
            "café €",
        ),
        (b"<meta charset=iso-8859-2><p>\xa3</p>".as_slice(), "Ł"),
    ] {
        assert!(values(&extract("html", bytes).unwrap()).contains(expected));
    }
    let bytes: Vec<u8> = [
        vec![0xff, 0xfe],
        "<p>UTF16 café</p>"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect(),
    ]
    .concat();
    assert!(values(&extract("html", &bytes).unwrap()).contains("UTF16 café"));
    for bytes in [
        b"<meta charset=unsupported><p>x</p>".as_slice(),
        b"<p>\xff</p>",
    ] {
        assert!(extract("html", bytes)
            .err()
            .unwrap()
            .to_string()
            .contains("ENCODING_UNSUPPORTED"));
    }
}

#[test]
fn projection_pdf_real_jpeg_flat_image_and_document_ratio() {
    use std::io::Write;
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
        .encode(&[255, 255, 255], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    let p = extract_with_pdfium(
        "pdf",
        &image_pdf(
            &["BT /F1 12 Tf 30 700 Td (JPEG_TEXT) Tj ET /Im1 Do"],
            "DCTDecode",
            jpeg,
        ),
        Some(&library()),
    )
    .unwrap();
    assert!(values(&p).contains("JPEG_TEXT"));
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&vec![255; 160_000]).unwrap();
    let flat = encoder.finish().unwrap();
    assert!(160_000 > flat.len() * 100);
    let bytes = image_pdf(
        &["/Im1 Do BT /F1 12 Tf 3 Tr 30 700 Td (FLAT_SCAN_OCR) Tj ET"],
        "FlateDecode",
        flat,
    );
    let mut doc = Document::load_mem(&bytes).unwrap();
    let mask=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>400,"Height"=>400,"ColorSpace"=>"DeviceGray","BitsPerComponent"=>8,"Filter"=>"JPXDecode"},vec![0;8]));
    let page = *doc.get_pages().values().next().unwrap();
    let resources = super::pdf::test_resources(&doc, page);
    let image_id = resources
        .get(b"XObject")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Im1")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(image_id)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("SMask", mask);
    let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("FLAT_SCAN_OCR") && n.kind.contains("invisible")));

    for balanced in [false, true] {
        let mut doc = Document::load_mem(&pdf(&[""], false)).unwrap();
        let mut content = b"BT /F1 12 Tf 30 700 Td (RATIO_TEXT) Tj ET\n".to_vec();
        content.extend(vec![b' '; 100_000]);
        let mut compressed = Stream::new(dictionary! {}, content);
        compressed.compress().unwrap();
        let first = doc.add_object(compressed);
        let mut contents = vec![Object::Reference(first)];
        if balanced {
            let second = doc.add_object(Stream::new(
                dictionary! {},
                format!("%{}\n", "x".repeat(4_000)).into_bytes(),
            ));
            contents.push(Object::Reference(second));
        } else {
            let unused = doc.add_object(Stream::new(
                dictionary! {"Subtype"=>"Image"},
                vec![0; 100_000],
            ));
            doc.catalog_mut().unwrap().set("UnusedImage", unused);
        }
        let page = *doc.get_pages().values().next().unwrap();
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Contents", contents);
        let result = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library()));
        if balanced {
            assert!(values(&result.unwrap()).contains("RATIO_TEXT"));
        } else {
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("EXPANSION_RATIO_EXCEEDED"));
        }
    }
}

#[test]
fn projection_pdf_xref_object_number_mismatch_has_precise_reason() {
    let mut bytes = pdf(&["BT /F1 12 Tf 30 700 Td (TEXT) Tj ET"], false);
    let doc = Document::load_mem(&bytes).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let lopdf::xref::XrefEntry::Normal { offset, .. } = doc.reference_table.get(page.0).unwrap()
    else {
        panic!("classic fixture")
    };
    let old = format!("{offset:010} 00000 n ");
    let at = bytes
        .windows(old.len())
        .position(|w| w == old.as_bytes())
        .unwrap();
    bytes[at..at + 10].copy_from_slice(b"0000000009");
    let error = extract_with_pdfium("pdf", &bytes, None).err().unwrap();
    assert!(
        error
            .to_string()
            .contains("PROJECTION_PDF_XREF_OBJECT_MISMATCH"),
        "{error:#}"
    );
}

#[test]
fn projection_pdf_form_cycle_refuses_before_pdfium() {
    let mut doc = Document::load_mem(&pdf(
        &["BT /F1 12 Tf 30 700 Td (SURVIVING_TEXT) Tj ET /Cycle Do"],
        false,
    ))
    .unwrap();
    let form=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),612.into(),792.into()]},b"/Again Do".to_vec()));
    doc.get_object_mut(form)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set(
            "Resources",
            dictionary! {"XObject"=>dictionary!{"Again"=>form}},
        );
    let page = *doc.get_pages().values().next().unwrap();
    let mut resources = super::pdf::test_resources(&doc, page);
    resources.set("XObject", dictionary! {"Cycle"=>form});
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", resources);
    let error = extract_with_pdfium("pdf", &save_pdf(doc), None)
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("Form depth/cycle"),
        "{error:#}"
    );
}

#[test]
fn projection_pdf_unmapped_glyph_replaced_and_counted() {
    let mut doc = Document::load_mem(&pdf(
        &["BT /F1 12 Tf 30 700 Td (A SURVIVING_TEXT) Tj ET"],
        false,
    ))
    .unwrap();
    // A ToUnicode mapping to an isolated UTF-16 surrogate has a real glyph
    // but no Unicode scalar. Keep neighboring readable text in the document.
    let cmap=doc.add_object(Stream::new(dictionary!{},b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /BadUnicode def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <41> <D800> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end".to_vec()));
    doc.get_object_mut((4, 0))
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("ToUnicode", cmap);
    let bytes = save_pdf(doc);
    let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
    assert!(values(&p).contains("SURVIVING_TEXT"));
    assert!(p.drops["unmapped-glyphs"] > 0);
    assert!(values(&p).contains('\u{fffd}'));
}

#[test]
fn projection_pdf_malformed_decoded_flate_and_unsupported_predictor_refuse() {
    for predictor in [1, 2] {
        let mut doc =
            Document::load_mem(&pdf(&["BT /F1 12 Tf 30 700 Td (TEXT) Tj ET"], false)).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let content = doc
            .get_object(page)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .as_reference()
            .unwrap();
        let stream = doc
            .get_object_mut(content)
            .unwrap()
            .as_stream_mut()
            .unwrap();
        stream.set_content(vec![0x78, 0x9c, 0xff, 0xff]);
        stream.dict.set("Filter", "FlateDecode");
        stream
            .dict
            .set("DecodeParms", dictionary! {"Predictor"=>predictor});
        let error = extract_with_pdfium("pdf", &save_pdf(doc), None)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(if predictor == 1 {
                "PROJECTION_PDF_MALFORMED"
            } else {
                "PROJECTION_PDF_STREAM_UNSUPPORTED"
            }),
            "{error}"
        );
    }
}

#[test]
fn projection_html_meta_utf16_and_bom_authority() {
    for label in ["utf-16", "utf-16le", "utf-16be"] {
        let source = format!("<meta charset='{label}'><body><p>ASCII body text declared as utf16.</p><script>SECRET</script><!--SECRET--><div hidden>HIDDEN</div>");
        let p = extract("html", source.as_bytes()).unwrap();
        assert_eq!(
            p.nodes
                .iter()
                .filter(|n| !n.hidden)
                .map(|n| n.text.as_str())
                .collect::<String>(),
            "ASCII body text declared as utf16."
        );
        assert!(!values(&p).contains("SECRET"));
    }
    for big_endian in [false, true] {
        let mut bytes = if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        };
        for c in "<meta charset='utf-8'><p>BOM body</p>".encode_utf16() {
            bytes.extend_from_slice(&if big_endian {
                c.to_be_bytes()
            } else {
                c.to_le_bytes()
            });
        }
        assert_eq!(values(&extract("html", &bytes).unwrap()), "BOM body");
    }
}

#[test]
fn projection_html_fallbacks_and_extra_blocks() {
    let p = extract("html", b"<p>Visible</p><progress>PROGRESS</progress><meter>METER</meter><details><summary>Sum</summary>Body</details><center>Cen</center><dialog open>Dialog</dialog><option>Opt</option>").unwrap();
    for text in ["PROGRESS", "METER"] {
        assert!(p.nodes.iter().any(|n| n.hidden && n.text == text));
    }
    let visible: Vec<_> = p
        .nodes
        .iter()
        .filter(|n| !n.hidden)
        .map(|n| n.text.as_str())
        .collect();
    assert_eq!(visible, ["Visible", "Sum", "Body", "Cen", "Dialog", "Opt"]);
}

// CC's craft2 probes, regenerated synthetically in memory. The encoder writes
// fixed blocks so even the 300 MiB test never allocates its expanded payload.
fn compressed_newlines(mib: usize) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    for _ in 0..mib * 16 {
        encoder.write_all(&[b'\n'; 65536]).unwrap();
    }
    encoder.finish().unwrap()
}

#[test]
fn projection_pdf_decode_caps_ignore_labels_and_resource_names() {
    for (role, name, label, mib) in [
        ("content", "", false, 60),
        ("content", "", true, 60),
        ("content", "", true, 300),
        ("form", "X1", false, 60),
        ("form", "A", false, 60),
        ("cmap", "A", true, 60),
    ] {
        let mut doc =
            Document::load_mem(&pdf(&["BT /F1 12 Tf 30 700 Td (BODY) Tj ET"], false)).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let mut dict = dictionary! {"Filter"=>"FlateDecode"};
        if label {
            dict.set("Subtype", "Image");
        }
        let mut resources = super::pdf::test_resources(&doc, page);
        if role == "form" {
            dict.set("Subtype", "Form");
            dict.set("Resources", resources.clone());
        }
        let bomb = doc.add_object(Stream::new(dict, compressed_newlines(mib)));
        match role {
            "content" => {
                // The same stream also has an image role: accounting must win.
                resources.set("XObject", dictionary! {"ImageAlias"=>bomb});
                doc.get_object_mut(page)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Contents", bomb);
            }
            "form" => {
                resources.set("XObject", dictionary! {name=>bomb});
                let content = doc.add_object(Stream::new(
                    dictionary! {},
                    format!("/{name} Do").into_bytes(),
                ));
                doc.get_object_mut(page)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Contents", content);
            }
            _ => {
                resources.set("Font", dictionary! {name=>dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "ToUnicode"=>bomb}});
            }
        }
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", resources);
        let error = super::pdf::metadata(&save_pdf(doc), &mut Projection::new()).unwrap_err();
        let expected = if mib == 300 {
            "PROJECTION_PART_BYTES_EXCEEDED"
        } else {
            "PROJECTION_EXPANSION_RATIO_EXCEEDED"
        };
        assert!(
            format!("{error:#}").contains(expected),
            "{role}/{name}/{label}: {error:#}"
        );
    }
}

#[test]
fn projection_pdf_actualtext_is_parsed_and_page_scoped() {
    for label in [false, true] {
        let mut doc = Document::load_mem(&pdf(&[
            "/Span << /Actual#54ext (REPLACEMENT) >> BDC BT /F1 12 Tf 30 700 Td (SHOWN) Tj ET EMC",
            "BT /F1 12 Tf 30 700 Td (ORDINARY) Tj ET",
        ], false)).unwrap();
        if label {
            let first = *doc.get_pages().values().next().unwrap();
            let content = doc.get_page_contents(first)[0];
            doc.get_object_mut(content)
                .unwrap()
                .as_stream_mut()
                .unwrap()
                .dict
                .set("Subtype", "Image");
        }
        let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
        assert!(p
            .nodes
            .iter()
            .any(|n| n.text.contains("REPLACEMENT") && n.kind.contains("ActualText")));
        assert!(p
            .nodes
            .iter()
            .any(|n| n.text.contains("ORDINARY") && !n.kind.contains("ActualText")));
    }
}

#[test]
fn projection_pdf_quartz_unused_zero_offset_entry() {
    for referenced in [false, true] {
        let mut doc =
            Document::load_mem(&pdf(&["BT /F1 12 Tf 30 700 Td (BODY) Tj ET"], false)).unwrap();
        let missing = doc.max_id + 1;
        if referenced {
            doc.catalog_mut().unwrap().set("Unused", (missing, 0));
        }
        let mut bytes = save_pdf(doc);
        let previous = Document::load_mem(&bytes).unwrap().xref_start;
        // Add an incremental xref entry without defining the object.
        let xref = bytes.len();
        bytes.extend_from_slice(format!("xref\n{missing} 1\n0000000000 00000 n \ntrailer\n<< /Size {} /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n", missing + 1).as_bytes());
        let result = super::pdf::metadata(&bytes, &mut Projection::new());
        if referenced {
            assert!(format!("{:#}", result.unwrap_err()).contains("XREF_OBJECT_MISMATCH"));
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn projection_pdf_reversed_media_and_larger_crop() {
    let mut doc = Document::load_mem(&pdf(
        &["BT /F1 12 Tf 30 700 Td (IN) Tj ET BT /F1 12 Tf 400 700 Td (OUTSIDE) Tj ET"],
        false,
    ))
    .unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let dict = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
    dict.set("MediaBox", vec![300.into(), 792.into(), 0.into(), 0.into()]);
    dict.set("CropBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
    let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("OUTSIDE") && n.kind.contains("off-page")));
}

#[test]
fn projection_pdf_inline_images_have_named_refusal() {
    let bytes = pdf(
        &["BT /F1 12 Tf 30 700 Td (BODY) Tj ET BI /W 1 /H 1 /CS /G /BPC 8 ID x EI"],
        false,
    );
    let error = super::pdf::metadata(&bytes, &mut Projection::new()).unwrap_err();
    assert!(format!("{error:#}").contains("PROJECTION_PDF_INLINE_IMAGE_UNSUPPORTED"));
}

#[test]
fn projection_pdf_stream_aliases_cannot_bypass_accounting() {
    let mut doc = Document::load_mem(&pdf(&[""], false)).unwrap();
    let bomb = doc.add_object(Stream::new(
        dictionary! {"Subtype"=>"Image", "Filter"=>"FlateDecode"},
        compressed_newlines(1),
    ));
    let alias = doc.add_object(Object::Reference(bomb));
    let page = *doc.get_pages().values().next().unwrap();
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", alias);
    let error = super::pdf::metadata(&save_pdf(doc), &mut Projection::new()).unwrap_err();
    assert!(format!("{error:#}").contains("PROJECTION_EXPANSION_RATIO_EXCEEDED"));
}

#[test]
fn projection_pdf_actualtext_in_named_form_and_property_operands() {
    let mut doc = Document::load_mem(&pdf(
        &["/A Do", "BT /F1 12 Tf 30 700 Td (ORDINARY) Tj ET"],
        false,
    ))
    .unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let mut resources = super::pdf::test_resources(&doc, page);
    resources.set(
        "Properties",
        dictionary! {"Replacement"=>dictionary! {"ActualText"=>Object::string_literal("PROPERTY")}},
    );
    let form = doc.add_object(Stream::new(dictionary! {
        "Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),600.into(),800.into()], "Resources"=>resources.clone()
    }, b"/Span /Replacement DP /Span << /Actual#54ext (FORM_REPLACEMENT) >> BDC BT /F1 12 Tf 30 700 Td (SHOWN) Tj ET EMC".to_vec()));
    resources.set("XObject", dictionary! {"A"=>form});
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", resources);
    let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("FORM_REPLACEMENT") && n.kind.contains("ActualText")));
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text.contains("ORDINARY") && !n.kind.contains("ActualText")));
}

// Synthetic equivalents of CC r1/r2/r3/s1: no external fixture bytes.
#[test]
fn projection_pdf_all_accounted_content_routes_refuse_inline_and_bad_syntax() {
    for route in ["fallback", "indirect", "string", "glyph", "unused"] {
        for body in [b"BI /W 1 /H 1 ID x EI".as_slice(), b"BT (unterminated"] {
            let mut doc = Document::load_mem(&pdf(&["/Outer Do"], false)).unwrap();
            let inner = doc.add_object(Stream::new(dictionary! {"Subtype"=>"Form"}, body.to_vec()));
            let page = *doc.get_pages().values().next().unwrap();
            let mut resources = super::pdf::test_resources(&doc, page);
            let mut xobjects = dictionary! {"Outer"=>inner};
            match route {
                "fallback" => {
                    let outer = doc.add_object(Stream::new(dictionary! {"Subtype"=>"Form", "Resources"=>dictionary! {"Font"=>dictionary!{}}}, b"/Inner Do".to_vec()));
                    xobjects.set("Outer", outer);
                    xobjects.set("Inner", inner);
                }
                "indirect" | "string" => {
                    let subtype = if route == "indirect" {
                        Object::Reference(doc.add_object(Object::Name(b"Form".to_vec())))
                    } else {
                        Object::string_literal("Form")
                    };
                    doc.get_object_mut(inner)
                        .unwrap()
                        .as_stream_mut()
                        .unwrap()
                        .dict
                        .set("Subtype", subtype);
                }
                "glyph" => {
                    doc.get_object_mut(inner)
                        .unwrap()
                        .as_stream_mut()
                        .unwrap()
                        .dict
                        .remove(b"Subtype");
                    let procs = doc.add_object(dictionary! {"a"=>inner});
                    let font = doc.add_object(dictionary! {"Subtype"=>"Type3", "CharProcs"=>procs});
                    resources.set("Font", dictionary! {"T3"=>font});
                    xobjects = dictionary! {};
                }
                "unused" => {
                    xobjects = dictionary! {"Unused"=>inner};
                }
                _ => unreachable!(),
            }
            resources.set("XObject", xobjects);
            doc.get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Resources", resources);
            let error = extract_with_pdfium("pdf", &save_pdf(doc), None)
                .err()
                .unwrap();
            let reason = if body.starts_with(b"BI") {
                "PROJECTION_PDF_INLINE_IMAGE_UNSUPPORTED"
            } else {
                "PROJECTION_PDF_MALFORMED"
            };
            assert!(format!("{error:#}").contains(reason), "{route}: {error:#}");
        }
    }
}

fn invocation_pdf(width: usize, levels: usize, leaf_bytes: usize, pages: usize) -> Vec<u8> {
    let body = "/Top Do ".repeat(width);
    let mut doc = Document::load_mem(&pdf(&vec![body.as_str(); pages], false)).unwrap();
    let mut leaf = b"BT /F1 12 Tf 30 700 Td (FORM_TEXT) Tj ET".to_vec();
    leaf.resize(leaf.len().max(leaf_bytes), b' ');
    let mut form = doc.add_object(Stream::new(
        dictionary! {"Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),612.into(),792.into()]},
        leaf,
    ));
    for _ in 1..levels {
        form = doc.add_object(Stream::new(dictionary! {"Subtype"=>"Form", "Resources"=>dictionary! {"XObject"=>dictionary! {"Child"=>form}}}, "/Child Do ".repeat(width).into_bytes()));
    }
    for page in doc.get_pages().values() {
        let mut resources = super::pdf::test_resources(&doc, *page);
        resources.set("XObject", dictionary! {"Top"=>form});
        doc.get_object_mut(*page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", resources);
    }
    save_pdf(doc)
}

#[test]
fn projection_pdf_form_invocation_count_and_expanded_bytes_are_document_budgets() {
    for width in [12, 40] {
        let bytes = invocation_pdf(width, 7, 0, 1);
        let start = std::time::Instant::now();
        let error = extract_with_pdfium("pdf", &bytes, None).err().unwrap();
        assert!(
            format!("{error:#}").contains("PROJECTION_PDF_FORM_INVOCATIONS_EXCEEDED"),
            "{error:#}"
        );
        assert!(start.elapsed().as_secs_f64() < 1.0);
    }
    // Count is document-wide, including when each page is individually below it.
    let bytes = invocation_pdf(20_000, 1, 0, 6);
    let error = extract_with_pdfium("pdf", &bytes, None).err().unwrap();
    assert!(
        format!("{error:#}").contains("PROJECTION_PDF_FORM_INVOCATIONS_EXCEEDED"),
        "{error:#}"
    );
    let bytes = invocation_pdf(20_000, 1, 30_000, 1);
    let error = extract_with_pdfium("pdf", &bytes, None).err().unwrap();
    assert!(
        format!("{error:#}").contains("PROJECTION_DOCUMENT_BYTES_EXCEEDED"),
        "{error:#}"
    );
    let bytes = invocation_pdf(20_000, 1, 0, 1);
    let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
    assert!(values(&p).contains("FORM_TEXT"));
}

#[test]
fn projection_pdf_form_subtype_and_page_property_fallback_flag_actual_text() {
    for indirect in [false, true] {
        for operator in ["BDC", "DP"] {
            let mut doc = Document::load_mem(&pdf(&["/Outer Do"], false)).unwrap();
            let subtype = if indirect {
                Object::Reference(doc.add_object(Object::Name(b"Form".to_vec())))
            } else {
                Object::string_literal("Form")
            };
            let body = format!(
                "/Span /P1 {operator} BT /F1 12 Tf 30 700 Td (SHOWN) Tj ET {}",
                if operator == "BDC" { "EMC" } else { "" }
            );
            let form = doc.add_object(Stream::new(dictionary! {"Subtype"=>subtype, "BBox"=>vec![0.into(),0.into(),612.into(),792.into()], "Resources"=>dictionary! {}}, body.into_bytes()));
            let page = *doc.get_pages().values().next().unwrap();
            let mut resources = super::pdf::test_resources(&doc, page);
            resources.set("XObject", dictionary! {"Outer"=>form});
            resources.set("Properties", dictionary! {"P1"=>dictionary! {"ActualText"=>Object::string_literal("REPLACEMENT")}});
            doc.get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Resources", resources);
            let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
            assert!(p.nodes.iter().any(|n| n.kind.contains("ActualText")));
        }
    }
}

#[test]
fn projection_html_meta_x_user_defined_maps_to_windows_1252() {
    assert_eq!(
        values(&extract("html", b"<meta charset=x-user-defined><p>\x80\xe9</p>").unwrap()),
        "€é"
    );
}

#[test]
fn projection_pdf_removed_hyphen_marker_and_c1_controls_are_counted() {
    let mut doc =
        Document::load_mem(&pdf(&["BT /F1 12 Tf 30 700 Td (ABCD) Tj ET"], false)).unwrap();
    let cmap = doc.add_object(Stream::new(dictionary! {}, b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <00> <FF> endcodespacerange 4 beginbfchar <41> <0058> <42> <0002> <43> <007F> <44> <009F> endbfchar endcmap end end".to_vec()));
    doc.get_object_mut((4, 0))
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("ToUnicode", cmap);
    let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
    assert!(values(&p).contains("X\u{fffd}\u{fffd}"));
    assert_eq!(p.drops["unmapped-glyphs"], 3);
}

#[test]
fn projection_pdf_nul_delimited_inline_image_refuses_by_name() {
    let bytes = pdf(&["q\0BI\0/W 1 /H 1 ID x EI Q"], false);
    let error = extract_with_pdfium("pdf", &bytes, None).err().unwrap();
    assert!(format!("{error:#}").contains("PROJECTION_PDF_INLINE_IMAGE_UNSUPPORTED"));
}

#[test]
fn projection_pdf_type3_width_operators_are_valid_content_syntax() {
    for width in ["1000 0 d0", "1000 0 0 0 1000 1000 d1"] {
        let mut doc = Document::load_mem(&pdf(
            &["BT /F1 12 Tf 30 700 Td (d0 d1 VISIBLE) Tj ET BT /T3 12 Tf 30 600 Td (a) Tj ET"],
            false,
        ))
        .unwrap();
        let glyph = doc.add_object(Stream::new(
            dictionary! {},
            format!("{width} 0 0 500 500 re f").into_bytes(),
        ));
        let font = doc.add_object(dictionary! {
            "Type"=>"Font", "Subtype"=>"Type3", "FontBBox"=>vec![0.into(),0.into(),1000.into(),1000.into()],
            "FontMatrix"=>vec![0.001.into(),0.into(),0.into(),0.001.into(),0.into(),0.into()],
            "CharProcs"=>dictionary! {"a"=>glyph}, "Encoding"=>dictionary! {"Type"=>"Encoding", "Differences"=>vec![97.into(),Object::Name(b"a".to_vec())]},
            "FirstChar"=>97, "LastChar"=>97, "Widths"=>vec![1000.into()],
        });
        let page = *doc.get_pages().values().next().unwrap();
        let mut resources = super::pdf::test_resources(&doc, page);
        let mut fonts = resources.get(b"Font").unwrap().as_dict().unwrap().clone();
        fonts.set("T3", font);
        resources.set("Font", fonts);
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", resources);
        let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
        assert!(values(&p).contains("d0 d1 VISIBLE"));
    }
}

// The three PDFium execution routes from CC M5. The guard runs without loading
// PDFium on the large shapes, so regressions cannot exhaust the test runner.
fn routed_invocation_pdf(route: &str, width: usize, levels: usize, actual: bool) -> Vec<u8> {
    let mut doc = Document::load_mem(&pdf(&["/Top Do"], false)).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let mut resources = super::pdf::test_resources(&doc, page);
    let mut names = lopdf::Dictionary::new();
    let mut next = None;
    for level in (0..levels).rev() {
        let name = format!("L{level}");
        let body = match next {
            Some(_) => format!("/L{} Do ", level + 1).repeat(width),
            None if actual => "/Span /P1 BDC BT /F1 12 Tf 30 700 Td (SHOWN) Tj ET EMC".into(),
            None => "BT /F1 12 Tf 30 700 Td (LEAF) Tj ET".into(),
        };
        let mut dict =
            dictionary! {"Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),612.into(),792.into()]};
        if route != "parent" {
            let mut own = resources.clone();
            if let Some(id) = next {
                own.set("XObject", dictionary! {format!("L{}",level+1)=>id});
            }
            dict.set("Resources", own);
        }
        let form = doc.add_object(Stream::new(dict, body.into_bytes()));
        names.set(name, form);
        next = Some(form);
    }
    resources.set(
        "Properties",
        dictionary! {"P1"=>dictionary! {"ActualText"=>Object::string_literal("REPLACEMENT")}},
    );
    let top = if route == "parent" {
        let mut own = resources.clone();
        own.set("XObject", names);
        doc.add_object(Stream::new(
            dictionary! {"Subtype"=>"Form", "Resources"=>own},
            b"/L0 Do".to_vec(),
        ))
    } else {
        next.unwrap()
    };
    resources.set("XObject", dictionary! {"Top"=>top});
    if route == "split" {
        let second = doc.add_object(Stream::new(dictionary! {}, b"n".to_vec()));
        let first = doc.get_page_contents(page)[0];
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Contents", vec![first.into(), second.into()]);
    }
    if route.starts_with("glyph") {
        let glyph = doc.add_object(Stream::new(
            dictionary! {},
            format!("1000 0 d0 {}", "/Top Do ".repeat(width)).into_bytes(),
        ));
        let font = doc.add_object(dictionary! {
            "Type"=>"Font", "Subtype"=>"Type3", "FontBBox"=>vec![0.into(),0.into(),1000.into(),1000.into()],
            "FontMatrix"=>vec![0.001.into(),0.into(),0.into(),0.001.into(),0.into()],
            "CharProcs"=>dictionary! {"a"=>glyph}, "Encoding"=>dictionary! {"Type"=>"Encoding", "Differences"=>vec![97.into(),Object::Name(b"a".to_vec())]},
            "FirstChar"=>97, "LastChar"=>97, "Widths"=>vec![1000.into()], "Resources"=>resources.clone(),
        });
        let mut fonts = resources.get(b"Font").unwrap().as_dict().unwrap().clone();
        fonts.set("T3", font);
        resources.set("Font", fonts);
        let content = doc.get_page_contents(page)[0];
        let body = if route == "glyph-inherited" {
            let wrapper = doc.add_object(Stream::new(
                dictionary! {"Subtype"=>"Form"},
                b"BT (a) Tj ET".to_vec(),
            ));
            resources.set("XObject", dictionary! {"Top"=>top,"Wrapper"=>wrapper});
            b"BT /T3 12 Tf ET /Wrapper Do".to_vec()
        } else {
            b"BT /T3 12 Tf 30 600 Td (a) Tj ET".to_vec()
        };
        doc.get_object_mut(content)
            .unwrap()
            .as_stream_mut()
            .unwrap()
            .set_plain_content(body);
    }
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", resources);
    save_pdf(doc)
}

#[test]
fn projection_pdf_parent_split_and_type3_invocations_refuse_by_leading_code() {
    for (route, width, levels) in [
        ("parent", 12, 7),
        ("split", 12, 7),
        ("glyph", 20, 5),
        ("glyph", 40, 4),
        ("glyph-inherited", 40, 4),
    ] {
        let bytes = routed_invocation_pdf(route, width, levels, false);
        let start = std::time::Instant::now();
        let error = super::pdf::metadata(&bytes, &mut Projection::new()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "PROJECTION_PDF_FORM_INVOCATIONS_EXCEEDED",
            "{route}: {error:#}"
        );
        assert!(start.elapsed().as_secs_f64() < 1.0);
    }
}

#[test]
fn projection_pdf_parent_and_split_actualtext_are_flagged() {
    for route in ["parent", "split"] {
        let bytes = routed_invocation_pdf(route, 1, 1, true);
        let p = extract_with_pdfium("pdf", &bytes, Some(&library())).unwrap();
        assert!(
            p.nodes.iter().any(|n| n.kind.contains("ActualText")),
            "{route}"
        );
    }
}

#[test]
fn projection_pdf_shared_form_has_one_byte_charge_per_invocation() {
    let mut doc = Document::load_mem(&invocation_pdf(1, 1, 300_000, 1000)).unwrap();
    let pages = doc.get_pages();
    let resources = super::pdf::test_resources(&doc, *pages.values().next().unwrap());
    let parent = doc
        .get_object(*pages.values().next().unwrap())
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(parent)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", resources);
    for page in pages.values() {
        doc.get_object_mut(*page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .remove(b"Resources");
    }
    let bytes = save_pdf(doc);
    let mut p = Projection::new();
    assert_eq!(super::pdf::metadata(&bytes, &mut p).unwrap(), 1000);
    assert!(p.expanded_bytes < 310_000_000, "{}", p.expanded_bytes);
}

#[test]
fn projection_pdf_document_memo_keeps_page_property_fallback_context() {
    let mut doc = Document::load_mem(&pdf(&["/Top Do", "/Top Do"], false)).unwrap();
    let form=doc.add_object(Stream::new(dictionary! {"Subtype"=>"Form", "Resources"=>dictionary! {}, "BBox"=>vec![0.into(),0.into(),612.into(),792.into()]},b"/Span /P1 BDC BT /F1 12 Tf 30 700 Td (SHOWN) Tj ET EMC".to_vec()));
    for (index, page) in doc.get_pages().values().enumerate() {
        let mut resources = super::pdf::test_resources(&doc, *page);
        resources.set("XObject", dictionary! {"Top"=>form});
        if index == 1 {
            resources.set("Properties",dictionary! {"P1"=>dictionary! {"ActualText"=>Object::string_literal("REPLACEMENT")}});
        }
        doc.get_object_mut(*page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", resources);
    }
    let p = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
    let flagged = p
        .nodes
        .iter()
        .filter(|n| n.kind.contains("ActualText"))
        .collect::<Vec<_>>();
    assert!(!flagged.is_empty());
    assert!(
        flagged.iter().all(|n| n.location.contains("page[2]")),
        "{:?}",
        flagged
    );
}

#[test]
fn projection_pdf_type3_budget_multiplies_by_shown_codes() {
    for selection in ["Tf", "gs", "underflow"] {
        let mut doc = Document::load_mem(&routed_invocation_pdf("glyph", 8, 5, false)).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let mut resources = super::pdf::test_resources(&doc, page);
        let font = resources
            .get(b"Font")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"T3")
            .unwrap()
            .clone();
        resources.set(
            "ExtGState",
            dictionary! {"State"=>dictionary! {"Font"=>vec![font,12.into()]}},
        );
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", resources);
        let content = doc.get_page_contents(page)[0];
        let select = match selection {
            "gs" => "/State gs",
            "underflow" => "/T3 12 Tf Q",
            _ => "/T3 12 Tf",
        };
        doc.get_object_mut(content)
            .unwrap()
            .as_stream_mut()
            .unwrap()
            .set_plain_content(format!("BT {select} 30 600 Td [(a) 0 (aa)] TJ ET").into_bytes());
        let error = super::pdf::metadata(&save_pdf(doc), &mut Projection::new()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "PROJECTION_PDF_FORM_INVOCATIONS_EXCEEDED",
            "{selection}"
        );
    }
}

#[test]
fn projection_pdf_structural_backstop_charges_unresolved_do_names() {
    let mut doc = Document::load_mem(&routed_invocation_pdf("parent", 2, 1, false)).unwrap();
    for object in doc.objects.values_mut() {
        if let Object::Stream(stream) = object {
            if stream.content.windows(4).any(|v| v == b"LEAF") {
                stream.set_plain_content(b"/Unresolved Do /Unresolved Do".to_vec());
            }
        }
    }
    let error = super::pdf::metadata(&save_pdf(doc), &mut Projection::new()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "PROJECTION_PDF_FORM_INVOCATIONS_EXCEEDED"
    );
}

#[test]
fn projection_pdf_padded_operands_keep_actualtext_flags() {
    for operator in ["BDC", "DP", "Do"] {
        let mut doc =
            Document::load_mem(&pdf(&["BT /F1 12 Tf 30 700 Td (BODY) Tj ET"], false)).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let content = doc.get_page_contents(page)[0];
        let body = if operator == "Do" {
            let form = doc.add_object(Stream::new(dictionary! {"Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),612.into(),792.into()]}, b"/Span <</ActualText (REPLACEMENT)>> BDC BT /F1 12 Tf 30 700 Td (BODY) Tj ET EMC".to_vec()));
            let mut resources = super::pdf::test_resources(&doc, page);
            resources.set("XObject", dictionary! {"Top"=>form});
            doc.get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Resources", resources);
            b"/Decoy /Top Do".to_vec()
        } else {
            format!("0 /Span <</ActualText (REPLACEMENT)>> {operator} BT /F1 12 Tf 30 700 Td (BODY) Tj ET {}", if operator == "BDC" {"EMC"} else {""}).into_bytes()
        };
        doc.get_object_mut(content)
            .unwrap()
            .as_stream_mut()
            .unwrap()
            .set_plain_content(body);
        let projection = extract_with_pdfium("pdf", &save_pdf(doc), Some(&library())).unwrap();
        assert!(
            projection
                .nodes
                .iter()
                .any(|n| n.kind.contains("ActualText")),
            "{operator}"
        );
    }
}
