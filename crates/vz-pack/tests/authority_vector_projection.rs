//! Synthetic exclusion contracts; these do not establish corpus capacity.
use std::io::{Cursor, Write};
use vz_core::office::projection::{extract, Projection};
fn package(kind: &str, parts: &[(&str, &str)]) -> Vec<u8> {
    let main = match kind {
        "docx" => "word/document.xml",
        "xlsx" => "xl/workbook.xml",
        _ => "ppt/presentation.xml",
    };
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("_rels/.rels", zip::write::SimpleFileOptions::default())
        .unwrap();
    write!(zip,"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"main\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"{main}\"/></Relationships>").unwrap();
    for (name, xml) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn text(p: &Projection) -> String {
    p.nodes
        .iter()
        .map(|n| n.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn projection_exclusion_canaries_docx() {
    let bytes = package(
        "docx",
        &[
            (
                "word/document.xml",
                r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>VISIBLE</w:t></w:r><w:del><w:r><w:delText>DELETE_CANARY</w:delText></w:r></w:del><w:moveFrom><w:r><w:t>MOVED_CANARY</w:t></w:r></w:moveFrom><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:sdt><w:sdtContent><w:r><w:t>INSTRUCTION_CANARY</w:t></w:r></w:sdtContent></w:sdt><w:r><w:instrText>FIELD_CANARY</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>DISPLAYED</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:fldSimple w:instr="SIMPLE_CANARY"><w:r><w:t>SIMPLE_DISPLAYED</w:t></w:r></w:fldSimple></w:p></w:body></w:document>"#,
            ),
            ("word/comments.xml", "<comments>COMMENT_CANARY</comments>"),
            (
                "docProps/core.xml",
                "<properties>PROPERTY_CANARY</properties>",
            ),
        ],
    );
    let p = extract("docx", &bytes).unwrap();
    let values = text(&p);
    assert!(values.contains("DISPLAYED") && values.contains("VISIBLE"));
    assert!(!values.contains("CANARY"));
    for key in [
        "deletions",
        "moved-from",
        "comments",
        "instructions",
        "properties",
    ] {
        assert!(p.drops[key] > 0, "{key}");
    }
    assert_eq!(p.drops["hidden-form-data"], 0);
}
#[test]
fn projection_exclusion_canaries_docx_field_and_moved_range_alt_text() {
    let bytes = package(
        "docx",
        &[(
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><w:body><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:sdt><w:sdtContent><wp:docPr descr="FIELD_ALT_CANARY" title="FIELD_TITLE_CANARY"/></w:sdtContent></w:sdt><w:r><w:fldChar w:fldCharType="separate"/><w:t>DISPLAYED</w:t><w:fldChar w:fldCharType="end"/></w:r><w:moveFromRangeStart w:id="1"/><wp:docPr descr="MOVED_ALT_CANARY"/><w:moveFromRangeEnd w:id="1"/><wp:docPr descr="CURRENT_ALT"/></w:p></w:body></w:document>"#,
        )],
    );
    let p = extract("docx", &bytes).unwrap();
    assert!(!text(&p).contains("CANARY"));
    assert!(text(&p).contains("DISPLAYED") && text(&p).contains("CURRENT_ALT"));
    assert_eq!(p.drops["instructions"], 3);
    assert_eq!(p.drops["moved-from"], 2);
}

#[test]
fn projection_hidden_seller_choice_docx() {
    let p=extract("docx",&package("docx",&[("word/document.xml",r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:rPr><w:vanish/></w:rPr><w:t>HIDDEN</w:t></w:r><w:r><w:t>VISIBLE</w:t></w:r></w:p></w:body></w:document>"#)])).unwrap();
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text == "HIDDEN" && n.hidden && n.location.contains("word/document.xml")));
    assert!(p.nodes.iter().any(|n| n.text == "VISIBLE" && !n.hidden));
}
fn workbook(state: &str, cells: &str) -> Vec<u8> {
    package(
        "xlsx",
        &[
            (
                "xl/workbook.xml",
                &format!(
                    r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet" sheetId="1" r:id="sheet" state="{state}"/></sheets><definedNames><definedName name="secret">NAME_FORMULA_CANARY</definedName></definedNames></workbook>"#
                ),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="sheet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                &format!(
                    r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{cells}</worksheet>"#
                ),
            ),
        ],
    )
}
#[test]
fn projection_exclusion_canaries_xlsx_cached_values_only() {
    let p=extract("xlsx",&workbook("visible",r#"<sheetData><row r="1"><c r="A1"><f>FORMULA_CANARY</f><v>42</v></c><c r="B1" t="e"><f>ERROR_FORMULA_CANARY</f><v>#DIV/0!</v></c></row></sheetData>"#)).unwrap();
    assert!(!text(&p).contains("CANARY"));
    assert!(text(&p).contains("42") && text(&p).contains("#DIV/0!"));
    assert_eq!(p.drops["formulas"], 3);
    assert!(!p.warnings.is_empty());
    let err = extract(
        "xlsx",
        &workbook(
            "visible",
            r#"<sheetData><row r="1"><c r="A1"><f>NO_CACHE</f></c></row></sheetData>"#,
        ),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(err.contains("PROJECTION_MISSING_CACHE") && err.contains("A1"));
}
#[test]
fn projection_hidden_seller_choice_xlsx_sheet_row_column() {
    for state in ["hidden", "veryHidden"] {
        let p=extract("xlsx",&workbook(state,r#"<sheetData><row r="1"><c r="A1" t="str"><v>SHEET_HIDDEN</v></c></row></sheetData>"#)).unwrap();
        assert!(p.nodes.iter().all(|n| n.hidden));
    }
    let p=extract("xlsx",&workbook("visible",r#"<cols><col min="2" max="2" hidden="1"/></cols><sheetData><row r="1" hidden="1"><c r="A1" t="str"><v>ROW_HIDDEN</v></c></row><row r="2"><c r="B2" t="str"><v>COL_HIDDEN</v></c><c r="A2" t="str"><v>VISIBLE</v></c></row></sheetData>"#)).unwrap();
    assert!(p
        .nodes
        .iter()
        .filter(|n| n.text.ends_with("HIDDEN"))
        .all(|n| n.hidden));
    assert!(p.nodes.iter().any(|n| n.text == "VISIBLE" && !n.hidden));
}
#[test]
fn projection_hidden_seller_choice_pptx_and_notes() {
    let bytes = package(
        "pptx",
        &[
            (
                "ppt/presentation.xml",
                r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="1" r:id="slide"/></p:sldIdLst></p:presentation>"#,
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="slide" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#,
            ),
            (
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" show="0"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>HIDDEN_SLIDE</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="notes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notes1.xml"/></Relationships>"#,
            ),
            (
                "ppt/notesSlides/notes1.xml",
                r#"<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:p><a:r><a:t>NOTES_CONTENT</a:t></a:r></a:p></p:notes>"#,
            ),
            (
                "ppt/comments/comment1.xml",
                "<comments>COMMENT_CANARY</comments>",
            ),
        ],
    );
    let p = extract("pptx", &bytes).unwrap();
    assert!(p.nodes.iter().all(|n| n.hidden));
    assert!(text(&p).contains("NOTES_CONTENT"));
    assert!(!text(&p).contains("COMMENT_CANARY"));
    assert!(p.drops["comments"] > 0);
}
#[test]
fn projection_format_limits() {
    for kind in ["pdf", "html", "htm"] {
        assert!(extract(kind, b"text").is_err());
    }
    assert!(extract("md", b"%PDF-hidden").is_err());
    assert!(extract("txt", b"PK\x03\x04").is_err());
    assert!(extract("txt", b"a\0b").is_err());
    assert!(extract(
        "docx",
        &package("docx", &[("word/document.xml", "<!DOCTYPE x><x/>")])
    )
    .is_err());
    assert!(extract("docx",&package("docx",&[("word/document.xml",r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:fldChar w:fldCharType="end"/></w:r></w:document>"#)])).is_err());
}

#[test]
fn projection_graphics_static_cache_and_pair_binding() {
    let chart_host = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:drawing><a:graphic><a:graphicData><c:chart r:id="chart"/></a:graphicData></a:graphic></w:drawing></w:r></w:p></w:body></w:document>"#;
    let chart_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="chart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/></Relationships>"#;
    let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:title><c:tx><c:rich xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:p><a:r><a:t>Annual</a:t></a:r><a:br/><a:r><a:t>Rev</a:t></a:r><a:r><a:t>enue</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:tx><c:strRef><c:f>CHART_FORMULA_CANARY</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>STATIC_TITLE</c:v></c:pt></c:strCache></c:strRef></c:tx><c:val><c:numRef><c:f>VALUE_FORMULA_CANARY</c:f><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>42</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let p = extract(
        "docx",
        &package(
            "docx",
            &[
                ("word/document.xml", chart_host),
                ("word/_rels/document.xml.rels", chart_rels),
                ("word/charts/chart1.xml", chart),
            ],
        ),
    )
    .unwrap();
    assert!(text(&p).contains("STATIC_TITLE") && text(&p).contains("42"));
    assert!(!text(&p).contains("CANARY"));
    assert_eq!(p.drops["formulas"], 2);
    assert!(!p.warnings.is_empty());
    assert!(text(&p).contains("Annual\nRevenue"));
    let missing = chart.replace("<c:ptCount val=\"1\"/>", "<c:ptCount val=\"2\"/>");
    assert!(extract(
        "docx",
        &package(
            "docx",
            &[
                ("word/document.xml", chart_host),
                ("word/_rels/document.xml.rels", chart_rels),
                ("word/charts/chart1.xml", &missing)
            ]
        )
    )
    .is_err());
    let host = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:drawing><a:graphic><a:graphicData><dgm:relIds r:dm="model"/></a:graphicData></a:graphic></w:drawing></w:r></w:p></w:body></w:document>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="model" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="diagrams/data1.xml"/></Relationships>"#;
    let model = r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram"><dgm:ptLst><dgm:pt modelId="node"><dgm:t><a:p><a:r><a:t>PAIRED_TEXT</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst><dsp:dataModelExt relId="cache"/></dgm:dataModel>"#;
    let model_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="cache" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="drawing1.xml"/></Relationships>"#;
    let drawing = r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:sp modelId="node"><dsp:nvSpPr><dsp:cNvPr hidden="1" descr="SMARTART_ALT_CANARY" title="SMARTART_TITLE_CANARY"/></dsp:nvSpPr><a:p><a:r><a:t>PAIRED_TEXT</a:t></a:r></a:p></dsp:sp></dsp:drawing>"#;
    for paired in [true, false] {
        let drawing = if paired {
            drawing.into()
        } else {
            drawing.replace("PAIRED_TEXT", "MISMATCH")
        };
        let bytes = package(
            "docx",
            &[
                ("word/document.xml", host),
                ("word/_rels/document.xml.rels", rels),
                ("word/diagrams/data1.xml", model),
                ("word/diagrams/_rels/data1.xml.rels", model_rels),
                ("word/diagrams/drawing1.xml", &drawing),
            ],
        );
        let result = extract("docx", &bytes);
        if paired {
            let p = result.unwrap();
            assert_eq!(p.nodes.len(), 3);
            assert!(p.nodes.iter().all(|n| n.hidden));
            for marker in ["SMARTART_ALT_CANARY", "SMARTART_TITLE_CANARY"] {
                assert!(p
                    .nodes
                    .iter()
                    .any(|n| n.text == marker && n.kind == "SmartArt alt text"));
            }
            assert_eq!(p.nodes[0].text, "PAIRED_TEXT");
            assert!(
                p.nodes[0].location.contains("/model/node!")
                    && p.nodes[0].location.contains("/shape/node")
            );
        } else {
            assert!(result.err().unwrap().to_string().contains("PAIR_AMBIGUOUS"));
        }
    }
    let host_rels = rels.replace("</Relationships>", r#"<Relationship Id="cache" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="diagrams/drawing1.xml"/></Relationships>"#);
    let p = extract(
        "docx",
        &package(
            "docx",
            &[
                ("word/document.xml", host),
                ("word/_rels/document.xml.rels", &host_rels),
                ("word/diagrams/data1.xml", model),
                ("word/diagrams/drawing1.xml", drawing),
            ],
        ),
    )
    .unwrap();
    assert!(text(&p).contains("SMARTART_ALT_CANARY"));
}

fn word(body: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body>{body}</w:body></w:document>"#
    );
    let mut parts = vec![("word/document.xml", xml.as_str())];
    parts.extend_from_slice(extra);
    package("docx", &parts)
}
fn deck(slide: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let xml = format!(
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main"><p:cSld><p:spTree>{slide}</p:spTree></p:cSld></p:sld>"#
    );
    let mut parts = vec![
        (
            "ppt/presentation.xml",
            r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="1" r:id="slide"/></p:sldIdLst></p:presentation>"#,
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="slide" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#,
        ),
        ("ppt/slides/slide1.xml", xml.as_str()),
    ];
    parts.extend_from_slice(extra);
    package("pptx", &parts)
}
#[test]
fn projection_deleted_rows_cells_and_hidden_signals() {
    let p = extract("docx", &word(r#"<w:p><w:r><w:t>VISIBLE</w:t></w:r></w:p><w:tbl><w:tr><w:trPr><w:del/></w:trPr><w:tc><w:p><w:r><w:t>ROW_DELETE_CANARY</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:cellDel/></w:tcPr><w:p><w:r><w:t>CELL_DELETE_CANARY</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:trPr><w:hidden/></w:trPr><w:tc><w:p><w:r><w:t>HIDDEN_ROW</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:drawing><wp:docPr hidden="1" descr="HIDDEN_DRAWING"/><pic:pic><pic:nvPicPr><pic:cNvPr descr="PICTURE_ALT_CANARY" title="PICTURE_TITLE_CANARY"/></pic:nvPicPr></pic:pic></w:drawing></w:r></w:p>"#, &[])).unwrap();
    assert!(!text(&p).contains("DELETE_CANARY"));
    assert_eq!(p.drops["deletions"], 2);
    for value in [
        "HIDDEN_ROW",
        "HIDDEN_DRAWING",
        "PICTURE_ALT_CANARY",
        "PICTURE_TITLE_CANARY",
    ] {
        assert!(
            p.nodes.iter().any(|n| n.text == value && n.hidden),
            "{value}"
        );
    }
    assert!(p
        .nodes
        .iter()
        .filter(|n| n.text.starts_with("PICTURE_"))
        .all(|n| n.kind == "alt text" && n.location.contains("/@")));
}
#[test]
fn projection_paragraph_run_boundaries_and_controls() {
    let p = extract("docx", &word(r#"<w:p><w:r><w:t>con</w:t></w:r><w:r><w:rPr><w14:ligatures w14:val="standard"/></w:rPr><w:t>fidential agree</w:t></w:r><w:r><w:t>ment</w:t><w:tab/><w:t>Ac</w:t></w:r><w:r><w:t>me</w:t><w:br/><w:t>next</w:t></w:r></w:p><w:p><w:r><w:t>second</w:t></w:r></w:p>"#, &[])).unwrap();
    assert_eq!(text(&p), "confidential agreement\tAcme\nnext\nsecond");
    assert_eq!(p.nodes.len(), 2);
    assert!(p.nodes[0].location.contains("/p[") && p.nodes[0].kind == "paragraph");
    let p = extract("pptx", &deck(r#"<p:sp><p:txBody><a:p><a:r><a:t>con</a:t></a:r><a:r><a:t>fidential agree</a:t></a:r><a:r><a:t>ment Ac</a:t></a:r><a:r><a:t>me</a:t></a:r><a:br/><a:r><a:t>next</a:t></a:r></a:p></p:txBody></p:sp>"#, &[])).unwrap();
    assert_eq!(text(&p), "confidential agreement Acme\nnext");
    assert_eq!(p.nodes.len(), 1);
}
#[test]
fn projection_unread_custom_xml_and_relationship_selected_headers() {
    let p = extract("docx", &word(r#"<w:p><w:r><w:t>BODY</w:t></w:r></w:p><w:sectPr><w:headerReference r:id="head"/></w:sectPr>"#, &[
        ("word/_rels/document.xml.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="head" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="custom" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/></Relationships>"#),
        ("word/header1.xml", r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>USED_HEADER</w:t></w:r></w:p></w:hdr>"#),
        ("word/header9.xml", "UNREAD_HEADER_CANARY malformed XML"),
        ("customXml/item1.xml", "<?mso-contentType testing?><private>UNREAD_CANARY</private>"),
    ])).unwrap();
    assert!(text(&p).contains("USED_HEADER"));
    assert!(!text(&p).contains("CANARY"));
    assert_eq!(p.drops["unread-parts"], 2);
    assert!(p
        .omissions
        .iter()
        .any(|(name, why)| name == "customXml/item1.xml" && why == "unread-parts"));
}
#[test]
fn projection_instruction_only_fields_and_named_textbox_refusals() {
    let p = extract("docx", &word(r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/><w:instrText>XE CANARY</w:instrText><w:fldChar w:fldCharType="end"/><w:t>VISIBLE</w:t></w:r></w:p>"#, &[])).unwrap();
    assert_eq!(text(&p), "VISIBLE");
    assert_eq!(p.drops["instructions"], 2);
    let unproven = word(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/><w:instrText>DATE</w:instrText><w:fldChar w:fldCharType="end"/><w:t>VISIBLE</w:t></w:r></w:p>"#,
        &[],
    );
    assert!(extract("docx", &unproven)
        .err()
        .unwrap()
        .to_string()
        .contains("PROJECTION_FIELD_RESULT_UNPROVEN"));
    for body in [
        r#"<v:shape xmlns:v="urn:schemas-microsoft-com:vml"><v:textbox><w:txbxContent><w:p><w:r><w:t>TEXTBOX</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape>"#,
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Fallback><w:p><w:r><w:t>TEXTBOX</w:t></w:r></w:p></mc:Fallback></mc:AlternateContent>"#,
    ] {
        let err = extract("docx", &word(body, &[])).err().unwrap().to_string();
        assert!(err.contains("PROJECTION_WORD_TEXTBOX_UNSUPPORTED"), "{err}");
    }
}
#[test]
fn projection_pptx_placeholder_layout_and_creation_id() {
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="layout" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/layout.xml"/></Relationships>"#;
    let placeholder = r#"<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>AUTHORING_PROMPT_CANARY</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sldLayout>"#;
    for prompts_only in [true, false] {
        let layout = if prompts_only {
            placeholder.into()
        } else {
            placeholder.replace("<p:ph/>", "")
        };
        let bytes = deck(
            r#"<p:sp><p:nvSpPr><p:cNvPr><a:extLst><a:ext uri="creation"><a16:creationId id="1"/></a:ext></a:extLst></p:cNvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>SLIDE_TEXT</a:t></a:r></a:p></p:txBody></p:sp>"#,
            &[
                ("ppt/slides/_rels/slide1.xml.rels", rels),
                ("ppt/slideLayouts/layout.xml", &layout),
            ],
        );
        if prompts_only {
            assert_eq!(text(&extract("pptx", &bytes).unwrap()), "SLIDE_TEXT");
        } else {
            assert!(extract("pptx", &bytes)
                .err()
                .unwrap()
                .to_string()
                .contains("PROJECTION_LAYOUT_UNSUPPORTED"));
        }
    }
}
#[test]
fn projection_real_powerpoint_slide_and_notes() {
    let p = extract(
        "pptx",
        include_bytes!("fixtures/office-public/SampleShow.pptx"),
    )
    .unwrap();
    assert!(p.nodes.iter().any(|n| n.kind == "slide paragraph"));
    assert!(p.nodes.iter().any(|n| n.kind == "notes paragraph"));
}
#[test]
fn projection_xlsx_empty_cache_comments_extensions_and_dimensions() {
    let p = extract("xlsx", &workbook("visible", r#"<cols><col min="2" max="2" width="0"/></cols><sheetData><row r="1" ht="0"><c r="A1" t="str"><v>ZERO_HEIGHT</v></c></row><row r="2"><c r="A2" t="str"><f>FORMULA_CANARY</f><v></v></c><c r="B2" t="str"><v>ZERO_WIDTH</v></c></row></sheetData><headerFooter><oddHeader>HEADER_CANARY</oddHeader></headerFooter><extLst><ext xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule><x14:formula>RULE_FORMULA_CANARY</x14:formula></x14:cfRule></x14:conditionalFormatting></x14:conditionalFormattings><x14:sparklineGroups><x14:sparklineGroup xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><xm:f>SPARK_FORMULA_CANARY</xm:f></x14:sparklineGroup></x14:sparklineGroups></ext></extLst>"#)).unwrap();
    assert!(!text(&p).contains("CANARY"));
    assert!(p
        .nodes
        .iter()
        .filter(|n| n.text.starts_with("ZERO_"))
        .all(|n| n.hidden));
    assert_eq!(p.drops["comments"], 0);
    assert_eq!(p.drops["headers-footers"], 1);
    assert_eq!(p.drops["formulas"], 4);
    let bytes = workbook(
        "visible",
        r#"<extLst><ext><unknown><text>UNSUPPORTED</text></unknown></ext></extLst>"#,
    );
    assert!(extract("xlsx", &bytes)
        .err()
        .unwrap()
        .to_string()
        .contains("content-bearing worksheet extension"));
}

#[test]
fn projection_xlsx_drawing_visibility_is_local_to_sheet_and_vml_comments_drop() {
    let bytes = package(
        "xlsx",
        &[
            (
                "xl/workbook.xml",
                r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Hidden" sheetId="1" r:id="s1" state="hidden"/><sheet name="Visible" sheetId="2" r:id="s2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/one.xml"/><Relationship Id="s2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/two.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/one.xml",
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="str"><v>HIDDEN_SHEET</v></c></row></sheetData></worksheet>"#,
            ),
            (
                "xl/worksheets/two.xml",
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><drawing r:id="drawing"/><legacyDrawing r:id="vml"/></worksheet>"#,
            ),
            (
                "xl/worksheets/_rels/two.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="drawing" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing.xml"/><Relationship Id="vml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="../drawings/comments.vml"/></Relationships>"#,
            ),
            (
                "xl/drawings/drawing.xml",
                r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:pic><xdr:blipFill><a:blip><a:extLst><a:ext uri="{28A0092B-C50C-407E-A947-70E740481C1C}"><a14:useLocalDpi xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" val="0"/></a:ext></a:extLst></a:blip></xdr:blipFill></xdr:pic><xdr:sp><xdr:txBody><a:p><a:r><a:t>VISIBLE_DRAWING</a:t></a:r></a:p></xdr:txBody></xdr:sp></xdr:wsDr>"#,
            ),
            (
                "xl/drawings/comments.vml",
                r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape><v:textbox>COMMENT_CANARY</v:textbox><x:ClientData ObjectType="Note"/></v:shape></xml>"#,
            ),
        ],
    );
    let p = extract("xlsx", &bytes).unwrap();
    assert!(p
        .nodes
        .iter()
        .any(|n| n.text == "VISIBLE_DRAWING" && !n.hidden));
    assert!(!text(&p).contains("CANARY"));
    assert_eq!(p.drops["comments"], 2);
}

#[test]
fn projection_tab_stops_are_not_run_tabs() {
    let p = extract("docx", &word(r#"<w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/><w:tab w:val="right" w:pos="1440"/></w:tabs></w:pPr><w:r><w:t>Hello</w:t><w:tab/><w:t>World</w:t></w:r></w:p><w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr></w:p>"#, &[])).unwrap();
    assert_eq!(text(&p), "Hello\tWorld");
    assert_eq!(p.nodes.len(), 1);
}

#[test]
fn projection_powerpoint_textless_extensions_and_text_refusal() {
    let body = r#"<p:sp><p:nvSpPr><p:nvPr><p:extLst><p:ext><p14:media xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"/></p:ext></p:extLst></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:rPr><a:hlinkClick><a:extLst><a:ext><a16:hlinkClr xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main"/></a:ext></a:extLst></a:hlinkClick></a:rPr><a:t>VISIBLE</a:t></a:r></a:p></p:txBody></p:sp>"#;
    assert_eq!(text(&extract("pptx", &deck(body, &[])).unwrap()), "VISIBLE");
    for payload in ["<a:t>EXTENSION_TEXT</a:t>", "literal text"] {
        let bad = body.replace("<p:ext>", &format!("<p:ext>{payload}"));
        assert!(extract("pptx", &deck(&bad, &[])).is_err());
    }
}

#[test]
fn projection_real_powerpoint_slide_level_creation_id_and_notes() {
    use std::io::Read;
    let mut source = zip::ZipArchive::new(Cursor::new(include_bytes!(
        "fixtures/office-public/SampleShow.pptx"
    )))
    .unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut changed = 0;
    for i in 0..source.len() {
        let mut entry = source.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if entry.name().starts_with("ppt/slides/slide") && entry.name().ends_with(".xml") {
            let xml = String::from_utf8(bytes).unwrap();
            assert!(xml.contains("</p:cSld>"));
            bytes = xml.replace("</p:cSld>", r#"<p:extLst><p:ext uri="{BB962C8B-B14F-4D97-AF65-F5344CB8AC3E}"><p14:creationId xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" val="244990882"/></p:ext></p:extLst></p:cSld>"#).into_bytes();
            changed += 1;
        }
        output
            .start_file(entry.name(), zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    assert!(changed > 0);
    let p = extract("pptx", &output.finish().unwrap().into_inner()).unwrap();
    assert!(p.nodes.iter().any(|n| n.kind == "slide paragraph"));
    assert!(p.nodes.iter().any(|n| n.kind == "notes paragraph"));
}

#[test]
fn projection_local_dpi_and_data_bar_id_metadata() {
    let drawing = r#"<w:p><w:r><w:drawing xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:blip><a:extLst><a:ext uri="{28A0092B-C50C-407E-A947-70E740481C1C}"><a14:useLocalDpi xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" val="0"/></a:ext></a:extLst></a:blip></w:drawing><w:t>VISIBLE</w:t></w:r></w:p>"#;
    assert_eq!(
        text(&extract("docx", &word(drawing, &[])).unwrap()),
        "VISIBLE"
    );
    assert!(extract(
        "docx",
        &word(&drawing.replace("useLocalDpi", "unknown"), &[])
    )
    .is_err());
    assert!(extract(
        "docx",
        &word(
            &drawing.replace("<a:ext uri=", "<a:ext>TEXT</a:ext><a:ext uri="),
            &[]
        )
    )
    .is_err());
    let rule = r#"<conditionalFormatting sqref="A1:A5"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/></dataBar><extLst><ext uri="{B025F937-C7B1-47D3-B67F-A62EFF666E3E}"><x14:id xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main">{GUID}</x14:id></ext></extLst></cfRule></conditionalFormatting>"#;
    assert_eq!(
        text(&extract("xlsx", &workbook("visible", rule)).unwrap()),
        "Sheet"
    );
    for bad in [
        rule.replace("type=\"dataBar\"", "type=\"expression\""),
        rule.replace("x14:id", "x14:dataValidations"),
        rule.replace("{GUID}", "<t>CONTENT</t>"),
    ] {
        assert!(extract("xlsx", &workbook("visible", &bad)).is_err());
    }
}
