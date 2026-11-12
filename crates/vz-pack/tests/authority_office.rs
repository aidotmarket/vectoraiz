//! Constructed OOXML adversarial fixtures. These are not Microsoft 365 corpus
//! qualification; installed Office/Document Inspector remains a manual gate.
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
};
use vz_core::{
    office::{self, Edit},
    pipeline::sha256,
};

const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
fn archive(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (k, v) in parts {
        z.start_file(
            k,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        z.write_all(v).unwrap();
    }
    z.finish().unwrap().into_inner()
}
fn members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut p = z.by_index(i).unwrap();
            let mut b = vec![];
            p.read_to_end(&mut b).unwrap();
            (p.name().into(), b)
        })
        .collect()
}
fn fixture(ext: &str) -> BTreeMap<String, Vec<u8>> {
    let(main,ty,body)=match ext{
        "docx"=>("word/document.xml","wordprocessingml.document",format!("<w:document xmlns:w=\"{W}\"><w:body><w:p><w:r><w:t>Project ORION retained body</w:t></w:r></w:p></w:body></w:document>")),
        "xlsx"=>("xl/workbook.xml","spreadsheetml.sheet",format!("<s:workbook xmlns:s=\"{S}\" xmlns:r=\"{R}\"><s:sheets><s:sheet name=\"Project ORION\" sheetId=\"1\" state=\"veryHidden\" r:id=\"s1\"/></s:sheets></s:workbook>")),
        _=>("ppt/presentation.xml","presentationml.presentation",format!("<p:presentation xmlns:p=\"{P}\" xmlns:r=\"{R}\"><p:sldIdLst><p:sldId id=\"256\" r:id=\"s1\"/></p:sldIdLst></p:presentation>")),
    };
    let mut parts=BTreeMap::from([
        ("[Content_Types].xml".into(),format!("<Types xmlns=\"{CT}\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Override PartName=\"/{main}\" ContentType=\"application/vnd.openxmlformats-officedocument.{ty}.main+xml\"/></Types>").into_bytes()),
        ("_rels/.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"root\" Type=\"{R}/officeDocument\" Target=\"{main}\"/></Relationships>").into_bytes()),
        (main.into(),body.into_bytes()),
    ]);
    if ext == "xlsx" {
        add(&mut parts,"xl/worksheets/identity-sheet.xml","application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",&format!("<s:worksheet xmlns:s=\"{S}\"><s:sheetData><s:row r=\"1\" hidden=\"1\"><s:c r=\"A1\" t=\"inlineStr\"><s:is><s:t>Project ORION retained body</s:t></s:is></s:c></s:row></s:sheetData></s:worksheet>"));
        parts.insert("xl/_rels/workbook.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"s1\" Type=\"{R}/worksheet\" Target=\"worksheets/identity-sheet.xml\"/></Relationships>").into_bytes());
    }
    if ext == "pptx" {
        add(&mut parts,"ppt/slides/slide1.xml","application/vnd.openxmlformats-officedocument.presentationml.slide+xml",&format!("<p:sld xmlns:p=\"{P}\" xmlns:a=\"{A}\" show=\"0\"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id=\"1\" name=\"Project ORION shape\" descr=\"Project ORION alt text\" hidden=\"1\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Project ORION retained body</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"));
        parts.insert("ppt/_rels/presentation.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"s1\" Type=\"{R}/slide\" Target=\"slides/slide1.xml\"/></Relationships>").into_bytes());
    }
    parts
}
fn add(parts: &mut BTreeMap<String, Vec<u8>>, path: &str, ty: &str, body: &str) {
    parts.insert(path.into(), body.as_bytes().to_vec());
    let ct = String::from_utf8(parts["[Content_Types].xml"].clone()).unwrap();
    parts.insert(
        "[Content_Types].xml".into(),
        ct.replace(
            "</Types>",
            &format!("<Override PartName=\"/{path}\" ContentType=\"{ty}\"/></Types>"),
        )
        .into_bytes(),
    );
}
fn rebuild(ext: &str, p: &BTreeMap<String, Vec<u8>>) -> (Vec<u8>, office::Report) {
    office::rebuild(&format!("document.{ext}"), &archive(p), &BTreeMap::new()).unwrap()
}
#[test]
fn office_benign_roundtrip() {
    for ext in ["docx", "xlsx", "pptx"] {
        let p = fixture(ext);
        let source = archive(&p);
        let digest = sha256(&source);
        let (bytes, r) = rebuild(ext, &p);
        let reopened = office::inspect(&format!("document.{ext}"), &bytes).unwrap();
        assert_eq!(r.inventory, reopened.inventory);
        assert_eq!(r.text(), reopened.text());
        assert!(r.text().contains("Project ORION retained body"));
        assert_eq!(sha256(&source), digest);
        assert!(!members(&bytes).keys().any(|p| p.contains("identity-sheet")));
    }
}
#[test]
fn office_metadata() {
    let markers = [
        "planted-user-S1927",
        "C:/planted-user-S1927/private/path",
        "aaaaaaaa-bbbb-cccc-dddd-123456789abc",
    ];
    for ext in ["docx", "xlsx", "pptx"] {
        let mut p = fixture(ext);
        add(
            &mut p,
            "docProps/core.xml",
            "application/vnd.openxmlformats-package.core-properties+xml",
            &format!("<private>{}</private>", markers.join("|")),
        );
        add(
            &mut p,
            "customXml/item1.xml",
            "application/xml",
            &format!("<private>{}</private>", markers.join("|")),
        );
        add(
            &mut p,
            "docMetadata/LabelInfo.xml",
            "application/xml",
            &format!("<private>{}</private>", markers.join("|")),
        );
        let carriers: &[&str] = match ext {
            "docx" => &[
                "docProps/app.xml",
                "docProps/custom.xml",
                "docProps/thumbnail.png",
                "word/comments.xml",
                "word/people.xml",
                "word/printerSettings/printer1.bin",
                "word/glossary/document.xml",
            ],
            "xlsx" => &[
                "docProps/app.xml",
                "docProps/custom.xml",
                "docProps/thumbnail.png",
                "xl/comments1.xml",
                "xl/threadedComments/threadedComment1.xml",
                "xl/persons/person.xml",
                "xl/printerSettings/printer1.bin",
                "xl/revisions/revisionLog.xml",
                "xl/webextensions/webextension1.xml",
            ],
            _ => &[
                "docProps/app.xml",
                "docProps/custom.xml",
                "docProps/thumbnail.png",
                "ppt/comments/comment1.xml",
                "ppt/commentAuthors.xml",
                "ppt/printerSettings/printer1.bin",
                "ppt/tags/tag1.xml",
                "ppt/revisionInfo.xml",
                "ppt/changesInfos/changes1.xml",
            ],
        };
        for carrier in carriers {
            add(
                &mut p,
                carrier,
                "application/xml",
                &format!("<private>{}</private>", markers.join("|")),
            );
        }
        if ext == "docx" {
            let source = String::from_utf8(p["word/document.xml"].clone()).unwrap();
            let revisions=format!("<w:ins w:author=\"{}\" w:date=\"2026-11-11T00:00:00Z\"><w:r><w:t>Accepted current insertion</w:t></w:r></w:ins><w:del w:author=\"{}\"><w:r><w:delText>{}</w:delText></w:r></w:del><w:commentRangeStart w:id=\"1\"/><w:commentRangeEnd w:id=\"1\"/>",markers[0],markers[0],markers[1]);
            p.insert(
                "word/document.xml".into(),
                source
                    .replace("</w:p>", &format!("{revisions}</w:p>"))
                    .into_bytes(),
            );
        }
        let rels = String::from_utf8(p["_rels/.rels"].clone()).unwrap();
        p.insert("_rels/.rels".into(), rels.replace("</Relationships>", &format!("<Relationship Id=\"properties\" Type=\"{REL}/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>")).into_bytes());
        let (bytes, r) = rebuild(ext, &p);
        assert!(r.removals.iter().any(|x| x.value.contains(markers[0])));
        for marker in markers {
            assert!(!bytes.windows(marker.len()).any(|v| v == marker.as_bytes()));
            for (k, v) in members(&bytes) {
                assert!(!k.contains(marker));
                assert!(!v.windows(marker.len()).any(|v| v == marker.as_bytes()));
            }
            assert!(!serde_json::to_string(&r.inventory)
                .unwrap()
                .contains(marker));
        }
        assert!(r.inventory.iter().any(|v| v.value.is_none()));
    }
}
#[test]
fn office_hidden() {
    for ext in ["docx", "xlsx", "pptx"] {
        let mut p = fixture(ext);
        if ext == "docx" {
            let b = String::from_utf8(p["word/document.xml"].clone())
                .unwrap()
                .replace("<w:r>", "<w:r><w:rPr><w:vanish/></w:rPr>");
            p.insert("word/document.xml".into(), b.into_bytes());
        }
        let (_, r) = rebuild(ext, &p);
        assert!(r
            .inventory
            .iter()
            .any(|v| v.value.as_deref().is_some_and(|s| s.contains("ORION"))));
        assert!(r.inventory.iter().filter(|v| v.content).all(|v| v.warning));
        assert!(r.inventory.iter().any(|v| v.location.contains(match ext {
            "docx" => "vanish",
            "xlsx" => "state",
            _ => "hidden",
        })));
    }
}
#[test]
fn office_formulas_cached_errors_and_uncached() {
    let mut p = fixture("xlsx");
    let path = "xl/worksheets/identity-sheet.xml";
    let original = String::from_utf8(p[path].clone()).unwrap();
    let formula = "<s:c r=\"B7\" t=\"e\"><s:f>NA()</s:f><s:v>#N/A</s:v></s:c>";
    p.insert(
        path.into(),
        original
            .replace("</s:row>", &format!("{formula}</s:row>"))
            .into_bytes(),
    );
    let (bytes, r) = rebuild("xlsx", &p);
    assert!(r
        .inventory
        .iter()
        .any(|v| v.value.as_deref() == Some("#N/A") && v.warning));
    assert!(r.removals.iter().any(|r| r.value.contains("NA()")));
    assert!(!String::from_utf8_lossy(&bytes).contains("NA()"));
    assert!(!r.inventory.iter().any(|v| v.location.contains("}f[")));
    p.insert(
        path.into(),
        original
            .replace("</s:row>", "<s:c r=\"B7\"><s:f>1+1</s:f></s:c></s:row>")
            .into_bytes(),
    );
    let e = office::rebuild("x.xlsx", &archive(&p), &BTreeMap::new())
        .err()
        .unwrap();
    assert!(e.to_string().contains("uncached formula"));
}
#[test]
fn office_shared_and_array_formula_removal_records() {
    let mut p = fixture("xlsx");
    p.insert("xl/worksheets/identity-sheet.xml".into(),format!("<s:worksheet xmlns:s=\"{S}\"><s:sheetData><s:row r=\"1\"><s:c r=\"A1\"><s:f t=\"shared\" si=\"0\" ref=\"A1:A2\">$B1+1</s:f><s:v>3</s:v></s:c><s:c r=\"C1\"><s:f t=\"array\" ref=\"C1:D1\">ROW(C1:D1)</s:f><s:v>1</s:v></s:c><s:c r=\"D1\"><s:v>1</s:v></s:c></s:row><s:row r=\"2\"><s:c r=\"A2\"><s:f t=\"shared\" si=\"0\"/><s:v>4</s:v></s:c></s:row></s:sheetData></s:worksheet>").into_bytes());
    let (bytes, r) = rebuild("xlsx", &p);
    assert!(r
        .removals
        .iter()
        .any(|v| v.location.contains("A2/formula") && v.value == "$B2+1"));
    assert!(r
        .removals
        .iter()
        .any(|v| v.location.contains("D1/formula") && v.value == "ROW(C1:D1)"));
    assert!(!String::from_utf8_lossy(&bytes).contains("ROW("));
}
#[test]
fn office_field_instructions() {
    let mut p = fixture("docx");
    let source = String::from_utf8(p["word/document.xml"].clone()).unwrap();
    let field="<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText>DDEAUTO planted-secret</w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>Safe displayed result</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>";
    p.insert(
        "word/document.xml".into(),
        source
            .replace("</w:p>", &format!("{field}</w:p>"))
            .into_bytes(),
    );
    let (bytes, r) = rebuild("docx", &p);
    assert!(r.text().contains("Safe displayed result"));
    assert!(!String::from_utf8_lossy(&bytes).contains("planted-secret"));
    assert!(r
        .removals
        .iter()
        .any(|r| r.value.contains("planted-secret")));
}
#[test]
fn office_edit_binding() {
    for ext in ["docx", "xlsx", "pptx"] {
        let p = fixture(ext);
        let input = archive(&p);
        let (_, r) = rebuild(ext, &p);
        let node = r
            .inventory
            .iter()
            .find(|v| {
                v.kind == "Office text" && v.value.as_deref() == Some("Project ORION retained body")
            })
            .unwrap();
        let edits = BTreeMap::from([(
            node.location.clone(),
            Edit {
                expected_sha256: sha256(node.value.as_ref().unwrap().as_bytes()),
                value: "Reviewed replacement body".into(),
            },
        )]);
        let (bytes, r) = office::rebuild(&format!("x.{ext}"), &input, &edits).unwrap();
        let reopened = office::inspect(&format!("x.{ext}"), &bytes).unwrap();
        assert!(reopened.text().contains("Reviewed replacement body"));
        assert_eq!(r.text(), reopened.text());
        for v in &reopened.inventory {
            if v.content {
                assert!(r.text().contains(v.value.as_deref().unwrap_or("")));
            }
        }
        let stale = BTreeMap::from([(
            node.location.clone(),
            Edit {
                expected_sha256: "0".repeat(64),
                value: "Wrong target".into(),
            },
        )]);
        assert!(office::rebuild(&format!("x.{ext}"), &input, &stale).is_err());
    }
}
#[test]
fn office_chunk_parity() {
    for ext in ["docx", "xlsx", "pptx"] {
        let mut p = fixture(ext);
        if ext == "docx" {
            let xml = String::from_utf8(p["word/document.xml"].clone())
                .unwrap()
                .replace(
                    "<w:p>",
                    "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>",
                );
            p.insert("word/document.xml".into(), xml.into_bytes());
        }
        let (_, report) = rebuild(ext, &p);
        let text = report.text();
        let chunks = vz_core::pipeline::chunk(
            "stable-doc",
            &[vz_core::pipeline::Page {
                page: 1,
                text: text.clone(),
                needs_ocr: false,
            }],
        );
        let content = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for v in report.inventory.iter().filter(|v| v.content) {
            assert!(content.contains(v.value.as_deref().unwrap_or("")));
        }
        for c in &chunks {
            assert!(text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains(&c.text));
        }
        if ext == "docx" {
            assert!(chunks
                .iter()
                .any(|c| c.heading_path.iter().any(|h| h.contains("ORION"))));
        }
    }
}
#[test]
fn office_identity_locations() {
    use vz_pack::sanitize::{rebuild_artifact, Changes};
    for ext in ["docx", "xlsx", "pptx"] {
        let input = archive(&fixture(ext));
        let changes = Changes {
            identity_terms: vec!["ORION".into(), "private-term-only-S1927".into()],
            ..Default::default()
        };
        let result = rebuild_artifact(&format!("x.{ext}"), &input, &changes).unwrap();
        assert!(result.inventory.iter().any(|e| e
            .value
            .as_deref()
            .is_some_and(|v| v.contains("ORION"))
            && e.warning
            && e.location.contains(&format!(".{ext}!"))));
        assert!(!String::from_utf8_lossy(&result.bytes).contains("private-term-only-S1927"));
    }
    assert!(vz_pack::sanitize::check_identifier("ORION-public-bucket", &["ORION".into()]).is_err());
}
#[test]
fn office_refusals() {
    for ext in ["docx", "xlsx", "pptx"] {
        let mut p = fixture(ext);
        p.insert(
            "word/vbaProject.bin".into(),
            b"disguised active payload".to_vec(),
        );
        assert!(office::rebuild(&format!("x.{ext}"), &archive(&p), &BTreeMap::new()).is_err());
        for path in ["../escape.xml", "WORD/document.xml", "escaped%2Exml"] {
            let mut p = fixture(ext);
            p.insert(path.into(), b"<x/>".to_vec());
            assert!(office::rebuild(&format!("x.{ext}"), &archive(&p), &BTreeMap::new()).is_err());
        }
        for variant in ["docm", "xlsm", "pptm", "doc", "xls", "ppt", "pdf", "html"] {
            assert!(office::rebuild(
                &format!("x.{variant}"),
                &archive(&fixture(ext)),
                &BTreeMap::new()
            )
            .is_err());
        }
        let mut p = fixture(ext);
        let k = p
            .keys()
            .find(|k| k.ends_with(".xml") && k.as_str() != "[Content_Types].xml")
            .unwrap()
            .clone();
        p.insert(
            k,
            b"<!DOCTYPE x [<!ENTITY secret SYSTEM 'file:///private'>]><x>&secret;</x>".to_vec(),
        );
        assert!(office::rebuild(&format!("x.{ext}"), &archive(&p), &BTreeMap::new()).is_err());
    }
}
#[test]
fn office_container_limits_in_rewrite_and_readback() {
    for ext in ["docx", "xlsx", "pptx"] {
        let source = archive(&fixture(ext));
        let mut encrypted = source.clone();
        encrypted[6] |= 1;
        let central = encrypted
            .windows(4)
            .position(|b| b == b"PK\x01\x02")
            .unwrap();
        encrypted[central + 8] |= 1;
        assert!(office::rebuild(&format!("x.{ext}"), &encrypted, &BTreeMap::new()).is_err());
        assert!(office::inspect(&format!("x.{ext}"), &encrypted).is_err());
        let mut too_large = source.clone();
        let central = too_large
            .windows(4)
            .position(|b| b == b"PK\x01\x02")
            .unwrap();
        too_large[central + 24..central + 28]
            .copy_from_slice(&((office::MAX_BYTES + 1) as u32).to_le_bytes());
        assert!(
            office::rebuild(&format!("x.{ext}"), &too_large, &BTreeMap::new())
                .err()
                .unwrap()
                .to_string()
                .contains("OFFICE_DOCUMENT_BYTES_EXCEEDED")
        );
        assert!(office::inspect(&format!("x.{ext}"), &too_large).is_err());
        let mut p = fixture(ext);
        let main = match ext {
            "docx" => "word/document.xml",
            "xlsx" => "xl/workbook.xml",
            _ => "ppt/presentation.xml",
        };
        p.insert(main.to_uppercase(), p[main].clone());
        assert!(
            office::rebuild(&format!("x.{ext}"), &archive(&p), &BTreeMap::new())
                .err()
                .unwrap()
                .to_string()
                .contains("duplicate")
        );
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..=office::MAX_PARTS {
        zip.start_file(
            format!("part{i}.xml"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    assert!(office::rebuild("x.docx", &bytes, &BTreeMap::new())
        .err()
        .unwrap()
        .to_string()
        .contains("OFFICE_PART_LIMIT_EXCEEDED"));
    assert!(office::inspect("x.docx", &bytes)
        .err()
        .unwrap()
        .to_string()
        .contains("OFFICE_PART_LIMIT_EXCEEDED"));
}
#[test]
fn office_external_links_and_template() {
    let mut p = fixture("docx");
    add(&mut p,"word/settings.xml","application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",&format!("<w:settings xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:attachedTemplate r:id=\"template\"/><w:docVars><w:docVar w:name=\"private-variable\" w:val=\"private-template-user\"/></w:docVars></w:settings>"));
    p.insert("word/_rels/settings.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"template\" Type=\"{R}/attachedTemplate\" Target=\"file:///private/template\" TargetMode=\"External\"/></Relationships>").into_bytes());
    p.insert("word/_rels/document.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"template\" Type=\"{R}/attachedTemplate\" Target=\"file:///private/template\" TargetMode=\"External\"/></Relationships>").into_bytes());
    let (bytes, r) = rebuild("docx", &p);
    assert!(!String::from_utf8_lossy(&bytes).contains("file:///private/template"));
    assert!(r
        .removals
        .iter()
        .any(|v| v.value.contains("file:///private/template")));
    let source=String::from_utf8(p["word/document.xml"].clone()).unwrap().replace("<w:document ","<w:document xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" ");
    p.insert("word/document.xml".into(),source.replace("</w:p>","<w:hyperlink r:id=\"web\"><w:r><w:t>Retained link label</w:t></w:r></w:hyperlink></w:p>").into_bytes());
    p.insert("word/_rels/document.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"web\" Type=\"{R}/hyperlink\" Target=\"HTTPS://example.test/ORION\" TargetMode=\"External\"/></Relationships>").into_bytes());
    let (_, r) = rebuild("docx", &p);
    assert!(r
        .inventory
        .iter()
        .any(|v| v.value.as_deref() == Some("HTTPS://example.test/ORION") && v.warning));
    p.insert("word/_rels/document.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"web\" Type=\"{R}/hyperlink\" Target=\"file:///removed/private-user\" TargetMode=\"External\"/></Relationships>").into_bytes());
    let (bytes, r) = rebuild("docx", &p);
    assert!(r.text().contains("Retained link label"));
    assert!(!String::from_utf8_lossy(&bytes).contains("file:///removed/private-user"));
    p.insert("word/_rels/document.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"image\" Type=\"{R}/image\" Target=\"https://example.test/image.png\" TargetMode=\"External\"/></Relationships>").into_bytes());
    assert!(office::rebuild("x.docx", &archive(&p), &BTreeMap::new()).is_err());
}
#[test]
fn office_values_only_x14_rules() {
    let mut p = fixture("xlsx");
    let path = "xl/worksheets/identity-sheet.xml";
    let source = String::from_utf8(p[path].clone()).unwrap();
    let rules="<s:extLst><s:ext uri=\"{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}\"><x14:dataValidations count=\"2\"><x14:dataValidation type=\"custom\"><x14:formula1><xm:f>private_x14_validation()</xm:f></x14:formula1><xm:sqref>B7</xm:sqref></x14:dataValidation><x14:dataValidation type=\"none\" prompt=\"Retained prompt ORION\"><xm:sqref>C7</xm:sqref></x14:dataValidation></x14:dataValidations></s:ext><s:ext uri=\"{78C0D931-6437-407D-A8EE-F0AAD7539E65}\"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type=\"expression\" priority=\"1\"><xm:f>private_x14_conditional()</xm:f></x14:cfRule><xm:sqref>B7</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></s:ext></s:extLst>";
    let source=source.replace("<s:worksheet ", "<s:worksheet xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\" ");
    p.insert(
        path.into(),
        source
            .replace("</s:worksheet>", &format!("{rules}</s:worksheet>"))
            .into_bytes(),
    );
    let (bytes, r) = rebuild("xlsx", &p);
    for value in ["private_x14_validation()", "private_x14_conditional()"] {
        assert!(!String::from_utf8_lossy(&bytes).contains(value));
        assert!(r.removals.iter().any(|v| v.value.contains(value)));
    }
    assert!(r
        .inventory
        .iter()
        .any(|v| v.value.as_deref() == Some("Retained prompt ORION")));
}

#[test]
fn office_pivot_cache() {
    let mut p = fixture("xlsx");
    add(
        &mut p,
        "xl/pivotCache/pivotCacheDefinition1.xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml",
        "<cacheSource>private-pivot-user</cacheSource>",
    );
    let path = "xl/worksheets/identity-sheet.xml";
    let source = String::from_utf8(p[path].clone()).unwrap();
    let rules="<s:dataValidations count=\"1\"><s:dataValidation type=\"custom\" sqref=\"B7\"><s:formula1>private_validation_formula()</s:formula1></s:dataValidation></s:dataValidations><s:conditionalFormatting sqref=\"B7\"><s:cfRule type=\"expression\" priority=\"1\"><s:formula>private_conditional_formula()</s:formula></s:cfRule></s:conditionalFormatting>";
    p.insert(
        path.into(),
        source
            .replace("</s:worksheet>", &format!("{rules}</s:worksheet>"))
            .into_bytes(),
    );
    let workbook=String::from_utf8(p["xl/workbook.xml"].clone()).unwrap().replace("</s:workbook>","<s:definedNames><s:definedName name=\"private_name\">private_defined_formula()</s:definedName></s:definedNames></s:workbook>");
    p.insert("xl/workbook.xml".into(), workbook.into_bytes());
    let (bytes, r) = rebuild("xlsx", &p);
    for term in [
        "private-pivot-user",
        "private_validation_formula",
        "private_conditional_formula",
        "private_defined_formula",
    ] {
        assert!(!String::from_utf8_lossy(&bytes).contains(term));
        assert!(r.removals.iter().any(|v| v.value.contains(term)));
    }
    assert!(!r.warnings.is_empty());
}

#[test]
fn office_shared_strings_are_cell_local_after_rebuild() {
    let mut p = fixture("xlsx");
    add(&mut p,"xl/sharedStrings.xml","application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml",&format!("<s:sst xmlns:s=\"{S}\" count=\"2\" uniqueCount=\"2\"><s:si><s:t>Shared identity</s:t></s:si><s:si><s:t>orphan-private-user</s:t></s:si></s:sst>"));
    p.insert("xl/worksheets/identity-sheet.xml".into(),format!("<s:worksheet xmlns:s=\"{S}\"><s:sheetData><s:row r=\"1\"><s:c r=\"A1\" t=\"s\"><s:v>0</s:v></s:c><s:c r=\"B1\" t=\"s\"><s:v>0</s:v></s:c></s:row></s:sheetData></s:worksheet>").into_bytes());
    let (bytes, r) = rebuild("xlsx", &p);
    assert!(!String::from_utf8_lossy(&bytes).contains("orphan-private-user"));
    let node = r
        .inventory
        .iter()
        .find(|v| {
            v.kind == "Office text"
                && v.location.contains("/cell/A1/")
                && v.value.as_deref() == Some("Shared identity")
        })
        .unwrap();
    let edits = BTreeMap::from([(
        node.location.clone(),
        Edit {
            expected_sha256: sha256(b"Shared identity"),
            value: "Only first cell".into(),
        },
    )]);
    let (_, edited) = office::edit("x.xlsx", &bytes, &edits).unwrap();
    assert!(
        edited
            .inventory
            .iter()
            .any(|v| v.location.contains("/cell/B1/")
                && v.value.as_deref() == Some("Shared identity"))
    );
    assert!(
        edited
            .inventory
            .iter()
            .any(|v| v.location.contains("/cell/A1/")
                && v.value.as_deref() == Some("Only first cell"))
    );
}

#[test]
fn office_notes_are_in_inventory_and_chunks() {
    let mut p = fixture("pptx");
    add(&mut p,"ppt/notesSlides/notesSlide1.xml","application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml",&format!("<p:notes xmlns:p=\"{P}\" xmlns:a=\"{A}\"><p:cSld><p:spTree><p:sp><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Hidden speaker note ORION</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"));
    p.insert("ppt/slides/_rels/slide1.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"notes\" Type=\"{R}/notesSlide\" Target=\"../notesSlides/notesSlide1.xml\"/></Relationships>").into_bytes());
    let (bytes, r) = rebuild("pptx", &p);
    assert!(r.text().contains("Hidden speaker note ORION"));
    assert!(r
        .inventory
        .iter()
        .any(|v| v.location.contains("notesSlides/")
            && v.value.as_deref() == Some("Hidden speaker note ORION")));
    office::inspect("x.pptx", &bytes).unwrap();
}

#[test]
fn office_identity_named_media_part() {
    // Decode a one-pixel PNG, attach source metadata/trailing private bytes,
    // then verify neutral path and intact image relationship after rebuilding.
    let png: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 0, 2, 0, 0, 5, 0,
        1, 122, 94, 171, 63, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    for ext in ["docx", "xlsx", "pptx"] {
        let mut p = fixture(ext);
        let prefix = match ext {
            "docx" => "word",
            "xlsx" => "xl",
            _ => "ppt",
        };
        let media_path = format!("{prefix}/media/planted-user-S1927/identity.png");
        let mut image = png.to_vec();
        image.extend_from_slice(b"planted-user-S1927 trailing private data");
        add(&mut p, &media_path, "image/png", "");
        p.insert(media_path.clone(), image);
        let main = match ext {
            "docx" => "document.xml",
            "xlsx" => "workbook.xml",
            _ => "presentation.xml",
        };
        let rp = format!("{prefix}/_rels/{main}.rels");
        let mut xml = p
            .get(&rp)
            .map(|v| String::from_utf8(v.clone()).unwrap())
            .unwrap_or_else(|| format!("<Relationships xmlns=\"{REL}\"></Relationships>"));
        xml=xml.replace("</Relationships>",&format!("<Relationship Id=\"media\" Type=\"{R}/image\" Target=\"media/planted-user-S1927/identity.png\"/></Relationships>"));
        p.insert(rp, xml.into_bytes());
        let (bytes, r) = rebuild(ext, &p);
        assert!(!String::from_utf8_lossy(&bytes).contains("planted-user-S1927"));
        assert!(r
            .inventory
            .iter()
            .any(|v| v.location.contains("/media/part-")));
        assert!(r.warnings.iter().any(|v| v.contains("visual review")));
    }
}
#[test]
fn office_capacity_chart_deck_specific_refusal() {
    let mut p = fixture("pptx");
    add(
        &mut p,
        "ppt/charts/chart1.xml",
        "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
        "<chart><cached>Varied chart points 1.25 3.5 7.75</cached></chart>",
    );
    let error = office::rebuild("chart-deck.pptx", &archive(&p), &BTreeMap::new())
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("unsupported Office part ppt/charts/chart1.xml"),
        "{error}"
    );
}

#[test]
fn office_xml_roundtrip_and_attribute_dos_bound() {
    let mut p = fixture("docx");
    let xml = String::from_utf8(p["word/document.xml"].clone())
        .unwrap()
        .replace("Project ORION retained body", "Fish &amp; chips &#x1F600;");
    p.insert("word/document.xml".into(), xml.into_bytes());
    let (_, r) = rebuild("docx", &p);
    assert!(r.text().contains("Fish & chips 😀"));
    let attrs = (0..129)
        .map(|i| format!(" attr{i}=\"x\""))
        .collect::<String>();
    p.insert(
        "word/document.xml".into(),
        format!("<w:document xmlns:w=\"{W}\"{attrs}><w:body/></w:document>").into_bytes(),
    );
    let err = office::rebuild("x.docx", &archive(&p), &BTreeMap::new())
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("OFFICE_XML_ATTRIBUTE_LIMIT_EXCEEDED"), "{err}");
}

#[test]
#[ignore = "million-cell supervised capacity run; invoke explicitly after building vz-parser-worker"]
fn office_capacity() {
    let mut p = fixture("xlsx");
    let mut sheet = format!("<s:worksheet xmlns:s=\"{S}\"><s:sheetData>");
    for row in 1..=50_000u64 {
        sheet.push_str(&format!("<s:row r=\"{row}\">"));
        for col in 0..20u64 {
            let letter = (b'A' + col as u8) as char;
            // Varied, non-repeating numeric values: do not accidentally make
            // the capacity fixture a >100:1 compression-ratio refusal.
            let value = row
                .wrapping_mul(6364136223846793005)
                .wrapping_add(col.wrapping_mul(1442695040888963407));
            sheet.push_str(&format!(
                "<s:c r=\"{letter}{row}\" t=\"n\"><s:v>{value}</s:v></s:c>"
            ));
        }
        sheet.push_str("</s:row>");
    }
    sheet.push_str("</s:sheetData></s:worksheet>");
    p.insert(
        "xl/worksheets/identity-sheet.xml".into(),
        sheet.into_bytes(),
    );
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("capacity.xlsx");
    std::fs::write(&input, archive(&p)).unwrap();
    drop(p);
    let request = vz_core::worker::Request {
        path: input,
        pdfium: None,
        office: Some(vz_core::worker::OfficeJob {
            name: "office_capacity.xlsx".into(),
            output: temp.path().into(),
            edits: BTreeMap::new(),
            readback: false,
            sanitized_input: false,
        }),
    };
    let result = vz_core::worker::run(
        &vz_core::worker::executable().unwrap(),
        &request,
        std::time::Duration::from_secs(120),
    );
    match result {
        Ok(_) => assert!(temp.path().join("output.bin").is_file()),
        Err(e) => {
            let message = format!("{e:#}");
            assert!(
                message.contains("OFFICE_MEMORY_LIMIT_EXCEEDED")
                    || message.contains("OFFICE_INVENTORY_LIMIT_EXCEEDED")
                    || message.contains("worker timeout"),
                "{message}"
            );
            eprintln!("office_capacity specific refusal: {message}");
            assert!(!temp.path().join("output.bin").exists());
        }
    }
}

#[test]
fn office_standard_font_theme_settings_and_tables() {
    let colors = [
        "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink",
    ];
    let palette = colors
        .iter()
        .map(|name| format!("<a:{name}><a:srgbClr val=\"336699\"/></a:{name}>"))
        .collect::<String>();
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let gradient="<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:satMod val=\"105000\"/><a:lumMod val=\"110000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"FFFFFF\"/></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill>";
    let line=format!("<a:ln w=\"9525\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\">{fill}<a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>");
    let effect="<a:effectStyle><a:effectLst><a:outerShdw blurRad=\"40000\" dist=\"20000\" dir=\"5400000\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"38000\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle>";
    let fonts=["majorFont","minorFont"].iter().map(|font|format!("<a:{font}><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/><a:font script=\"Arab\" typeface=\"Arial\"/></a:{font}>")).collect::<String>();
    let theme=format!("<a:theme xmlns:a=\"{A}\" name=\"Office\"><a:themeElements><a:clrScheme name=\"Office\">{palette}</a:clrScheme><a:fontScheme name=\"Office\">{fonts}</a:fontScheme><a:fmtScheme name=\"Office\"><a:fillStyleLst>{fill}{gradient}{fill}</a:fillStyleLst><a:lnStyleLst>{}</a:lnStyleLst><a:effectStyleLst>{}</a:effectStyleLst><a:bgFillStyleLst>{fill}{gradient}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>",line.repeat(3),effect.repeat(3));
    for ext in ["docx", "xlsx", "pptx"] {
        let mut p = fixture(ext);
        let prefix = match ext {
            "docx" => "word",
            "xlsx" => "xl",
            _ => "ppt",
        };
        add(
            &mut p,
            &format!("{prefix}/theme/theme1.xml"),
            "application/vnd.openxmlformats-officedocument.theme+xml",
            &theme,
        );
        if ext == "docx" {
            add(&mut p,"word/fontTable.xml","application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml",&format!("<w:fonts xmlns:w=\"{W}\"><w:font w:name=\"Calibri\"><w:panose1 w:val=\"020F0502020204030204\"/><w:charset w:val=\"00\"/><w:family w:val=\"swiss\"/><w:pitch w:val=\"variable\"/><w:sig w:usb0=\"E0002AFF\" w:usb1=\"C0007843\" w:usb2=\"00000009\" w:usb3=\"00000000\" w:csb0=\"000001FF\" w:csb1=\"00000000\"/></w:font></w:fonts>"));
            add(&mut p,"word/settings.xml","application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",&format!("<w:settings xmlns:w=\"{W}\" xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\"><w:zoom w:percent=\"100\"/><w:proofState w:spelling=\"clean\" w:grammar=\"clean\"/><w:defaultTabStop w:val=\"720\"/><w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat><m:mathPr><m:mathFont m:val=\"Cambria Math\"/></m:mathPr><w:themeFontLang w:val=\"en-US\"/><w:decimalSymbol w:val=\".\"/><w:listSeparator w:val=\",\"/></w:settings>"));
            add(&mut p,"word/styles.xml","application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",&format!("<w:styles xmlns:w=\"{W}\"><w:latentStyles w:defLockedState=\"0\" w:defUIPriority=\"99\" w:defSemiHidden=\"1\" w:defUnhideWhenUsed=\"1\" w:defQFormat=\"0\" w:count=\"1\"><w:lsdException w:name=\"Normal\" w:uiPriority=\"0\" w:qFormat=\"1\"/></w:latentStyles><w:style w:type=\"paragraph\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/><w:pPr><w:tabs><w:tab w:val=\"left\" w:pos=\"720\"/></w:tabs></w:pPr></w:style></w:styles>"));
        } else if ext == "xlsx" {
            let xml=String::from_utf8(p["xl/workbook.xml"].clone()).unwrap().replace("<s:sheets>","<s:fileVersion appName=\"xl\" lastEdited=\"7\" lowestEdited=\"7\" rupBuild=\"1234\"/><s:sheets>");
            p.insert("xl/workbook.xml".into(), xml.into_bytes());
        } else {
            add(&mut p,"ppt/tableStyles.xml","application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml",&format!("<a:tblStyleLst xmlns:a=\"{A}\" def=\"{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}\"/>"));
            add(&mut p,"ppt/presProps.xml","application/vnd.openxmlformats-officedocument.presentationml.presProps+xml",&format!("<p:presentationPr xmlns:p=\"{P}\"><p:showPr loop=\"0\"><p:present/><p:sldAll/></p:showPr></p:presentationPr>"));
            add(
                &mut p,
                "ppt/viewProps.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml",
                "<editor>private-editor-state</editor>",
            );
            let graphic="<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"2\" name=\"Table\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"1000000\" cy=\"1000000\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr firstRow=\"1\" bandRow=\"1\"><a:tableStyleId>{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}</a:tableStyleId></a:tblPr><a:tblGrid><a:gridCol w=\"1000000\"/></a:tblGrid><a:tr h=\"400000\"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Office table text ORION</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>".to_owned();
            let slide = String::from_utf8(p["ppt/slides/slide1.xml"].clone())
                .unwrap()
                .replace("</p:spTree>", &format!("{graphic}</p:spTree>"));
            p.insert("ppt/slides/slide1.xml".into(), slide.into_bytes());
        }
        let (bytes, report) = rebuild(ext, &p);
        assert!(report
            .inventory
            .iter()
            .any(|v| v.value.as_deref() == Some("Calibri")));
        assert!(!String::from_utf8_lossy(&bytes).contains("private-editor-state"));
        if ext == "pptx" {
            assert!(report.text().contains("Office table text ORION"));
        }
        let bad = theme.replace("<a:gradFill ", "<a:gradFill privatePath=\"forbidden\" ");
        p.insert(format!("{prefix}/theme/theme1.xml"), bad.into_bytes());
        assert!(office::rebuild(&format!("x.{ext}"), &archive(&p), &BTreeMap::new()).is_err());
    }
}

#[test]
fn office_neutral_sheet_directories_rewrite_relationships() {
    let mut p = fixture("xlsx");
    let old = "xl/worksheets/identity-sheet.xml";
    let renamed = "xl/worksheets/planted-user-S1927/private-sheet.xml";
    let data = p.remove(old).unwrap();
    p.insert(renamed.into(), data);
    let types = String::from_utf8(p["[Content_Types].xml"].clone())
        .unwrap()
        .replace(old, renamed);
    p.insert("[Content_Types].xml".into(), types.into_bytes());
    let rels = String::from_utf8(p["xl/_rels/workbook.xml.rels"].clone())
        .unwrap()
        .replace(
            "worksheets/identity-sheet.xml",
            "worksheets/planted-user-S1927/private-sheet.xml",
        );
    p.insert("xl/_rels/workbook.xml.rels".into(), rels.into_bytes());
    let (bytes, report) = rebuild("xlsx", &p);
    assert!(!String::from_utf8_lossy(&bytes).contains("planted-user-S1927"));
    assert!(report
        .inventory
        .iter()
        .any(|v| v.location.starts_with("xl/worksheets/part-")));
}

#[test]
fn office_standard_word_rendering_settings_refuse_active_neighbors() {
    let mut p = fixture("docx");
    let settings = format!(
        r#"<w:settings xmlns:w="{W}"><w:hideSpellingErrors/><w:stylePaneFormatFilter w:val="3F01"/><w:doNotAutoCompressPictures/><w:compat><w:useFELayout/><w:doNotUseHTMLParagraphAutoSpacing w:val="1"/><w:useWord2002TableStyleRules/></w:compat><w:footnotePr><w:numFmt w:val="decimal"/><w:footnote w:id="-1"/></w:footnotePr></w:settings>"#
    );
    add(
        &mut p,
        "word/settings.xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
        &settings,
    );
    let styles = format!(
        r#"<w:styles xmlns:w="{W}"><w:style w:type="table" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/></w:tblCellMar><w:tblLook w:val="04A0" w:firstRow="1" w:noVBand="1"/></w:tblPr><w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr></w:tblStylePr></w:style></w:styles>"#
    );
    add(
        &mut p,
        "word/styles.xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
        &styles,
    );
    // top/bottom are context-dependent table margins, not paragraph borders.
    let (bytes, report) = rebuild("docx", &p);
    assert_eq!(
        report.inventory,
        office::inspect("x.docx", &bytes).unwrap().inventory
    );
    for payload in [
        "<w:mailMerge/>",
        "<w:updateFields w:val=\"1\"/>",
        "<w:saveThroughXslt/>",
        "<w:unknownEditorSetting/>",
        "<w:useFELayout w:privatePath=\"secret\"/>",
    ] {
        p.insert(
            "word/settings.xml".into(),
            settings
                .replace("</w:settings>", &format!("{payload}</w:settings>"))
                .into_bytes(),
        );
        let error = office::rebuild("x.docx", &archive(&p), &BTreeMap::new())
            .err()
            .expect("active setting must refuse");
        assert!(format!("{error:#}").contains("word/settings.xml"));
    }
}

#[test]
fn formula_cfvo_rules_removed_with_private_evidence() {
    let mut p = fixture("xlsx");
    let path = "xl/worksheets/identity-sheet.xml";
    let sheet = String::from_utf8(p[path].clone()).unwrap();
    let rules = "<s:conditionalFormatting sqref=\"A1:A5\"><s:cfRule type=\"colorScale\" priority=\"1\"><s:colorScale><s:cfvo type=\"formula\" val=\"SUM(A1:A5)+1927\"/><s:cfvo type=\"max\"/></s:colorScale></s:cfRule><s:cfRule type=\"dataBar\" priority=\"2\"><s:dataBar><s:cfvo type=\"min\"/><s:cfvo type=\"max\"/></s:dataBar></s:cfRule></s:conditionalFormatting>";
    p.insert(
        path.into(),
        sheet
            .replace("</s:worksheet>", &format!("{rules}</s:worksheet>"))
            .into_bytes(),
    );
    let (bytes, report) = rebuild("xlsx", &p);
    assert!(report
        .removals
        .iter()
        .any(|r| r.value.contains("SUM(A1:A5)+1927")));
    for part in members(&bytes).values() {
        assert!(!String::from_utf8_lossy(part).contains("SUM(A1:A5)+1927"));
    }
    let reopened = office::inspect("document.xlsx", &bytes).unwrap();
    assert_eq!(report.inventory, reopened.inventory);
    assert!(members(&bytes)
        .values()
        .any(|b| String::from_utf8_lossy(b).contains("dataBar")));
    // Readback must independently reject a formula-valued threshold.
    assert!(office::inspect("document.xlsx", &archive(&p)).is_err());
}

#[test]
fn connected_excel_carriers_and_alternate_content_removed() {
    let mut p = fixture("xlsx");
    let workbook = String::from_utf8(p["xl/workbook.xml"].clone()).unwrap();
    let alternate = "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:x15ac=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac\"><mc:Choice Requires=\"x15ac\"><x15ac:absPath url=\"C:/planted-1927/private\"/></mc:Choice><mc:Fallback/></mc:AlternateContent>";
    p.insert(
        "xl/workbook.xml".into(),
        workbook
            .replace("</s:workbook>", &format!("{alternate}</s:workbook>"))
            .into_bytes(),
    );
    let path = "xl/worksheets/identity-sheet.xml";
    let sheet = String::from_utf8(p[path].clone()).unwrap();
    p.insert(path.into(),sheet.replace("<s:worksheet ",&format!("<s:worksheet xmlns:r=\"{R}\" ")).replace("</s:worksheet>","<s:pageSetup r:id=\"printer\" orientation=\"landscape\"/><s:legacyDrawing r:id=\"notes\"/></s:worksheet>").into_bytes());
    add(
        &mut p,
        "xl/printerSettings/printer1.bin",
        "application/octet-stream",
        "planted-1927-printer",
    );
    add(
        &mut p,
        "xl/comments1.xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml",
        "<comments>planted-1927-comment</comments>",
    );
    add(&mut p,"xl/drawings/vmlDrawing1.vml","application/vnd.openxmlformats-officedocument.vmlDrawing","<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><v:shape id=\"planted-1927\"><x:ClientData ObjectType=\"Note\"/></v:shape></xml>");
    p.insert("xl/worksheets/_rels/identity-sheet.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"printer\" Type=\"{R}/printerSettings\" Target=\"../printerSettings/printer1.bin\"/><Relationship Id=\"notes\" Type=\"{R}/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\"/><Relationship Id=\"comments\" Type=\"{R}/comments\" Target=\"../comments1.xml\"/></Relationships>").into_bytes());
    let (bytes, report) = rebuild("xlsx", &p);
    assert!(report
        .removals
        .iter()
        .any(|r| r.value.contains("planted-1927")));
    for b in members(&bytes).values() {
        assert!(!String::from_utf8_lossy(b).contains("planted-1927"));
    }
    assert_eq!(
        report.inventory,
        office::inspect("document.xlsx", &bytes).unwrap().inventory
    );
}

#[test]
fn embedded_font_bindings_removed() {
    let mut p = fixture("docx");
    add(&mut p,"word/fontTable.xml","application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml", &format!("<w:fonts xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:font w:name=\"Calibri\"><w:embedRegular r:id=\"font\" w:fontKey=\"planted-1927-key\"/></w:font></w:fonts>"));
    add(
        &mut p,
        "word/fonts/font1.odttf",
        "application/vnd.openxmlformats-officedocument.obfuscatedFont",
        "planted-1927-font",
    );
    p.insert("word/_rels/fontTable.xml.rels".into(),format!("<Relationships xmlns=\"{REL}\"><Relationship Id=\"font\" Type=\"{R}/font\" Target=\"fonts/font1.odttf\"/></Relationships>").into_bytes());
    let (bytes, _) = rebuild("docx", &p);
    for b in members(&bytes).values() {
        assert!(!String::from_utf8_lossy(b).contains("planted-1927"));
    }
}

#[test]
fn unmodified_office_saved_public_fixtures_and_planted_properties() {
    for (ext, input) in [
        (
            "docx",
            include_bytes!("fixtures/office-public/SampleDoc.docx").as_slice(),
        ),
        (
            "xlsx",
            include_bytes!("fixtures/office-public/WithVariousData.xlsx").as_slice(),
        ),
        (
            "pptx",
            include_bytes!("fixtures/office-public/SampleShow.pptx").as_slice(),
        ),
    ] {
        let digest = sha256(input);
        let (output, report) =
            office::rebuild(&format!("document.{ext}"), input, &BTreeMap::new()).unwrap();
        assert_eq!(
            report.inventory,
            office::inspect(&format!("document.{ext}"), &output)
                .unwrap()
                .inventory
        );
        assert!(!report.text().is_empty());
        assert_eq!(sha256(input), digest);
        let mut planted = members(input);
        let marker = "planted-public-fixture-S1927-tenant-1927";
        let properties = String::from_utf8(planted["docProps/core.xml"].clone()).unwrap();
        // Use an existing, realistically connected removal-only property part.
        let at = properties.rfind("</").unwrap();
        let properties = format!(
            "{}<private>{marker}</private>{}",
            &properties[..at],
            &properties[at..]
        );
        planted.insert("docProps/core.xml".into(), properties.into_bytes());
        let (bytes, report) = rebuild(ext, &planted);
        assert!(report.removals.iter().any(|r| r.value.contains(marker)));
        assert!(!bytes.windows(marker.len()).any(|w| w == marker.as_bytes()));
        for (path, member) in members(&bytes) {
            assert!(!path.contains(marker));
            assert!(!member.windows(marker.len()).any(|w| w == marker.as_bytes()));
        }
    }
}
