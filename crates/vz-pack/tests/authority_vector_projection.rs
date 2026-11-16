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
    let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:tx><c:strRef><c:f>CHART_FORMULA_CANARY</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>STATIC_TITLE</c:v></c:pt></c:strCache></c:strRef></c:tx><c:val><c:numRef><c:f>VALUE_FORMULA_CANARY</c:f><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>42</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
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
    let drawing = r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:sp modelId="node"><a:p><a:r><a:t>PAIRED_TEXT</a:t></a:r></a:p></dsp:sp></dsp:drawing>"#;
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
            assert_eq!(p.nodes.len(), 1);
            assert_eq!(p.nodes[0].text, "PAIRED_TEXT");
            assert!(
                p.nodes[0].location.contains("/model/node!")
                    && p.nodes[0].location.contains("/shape/node")
            );
        } else {
            assert!(result.err().unwrap().to_string().contains("PAIR_AMBIGUOUS"));
        }
    }
}
