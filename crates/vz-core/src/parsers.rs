//! Parser implementations run exclusively inside the supervised worker.
use crate::{pipeline::Page, worker::Request};
use anyhow::{bail, Context, Result};
use calamine::{open_workbook_auto, Reader};
use quick_xml::{events::Event, Reader as XmlReader};
use std::{fs::File, io::Read};

pub const MAX_TOTAL: u64 = 512_000_000;
pub const MAX_PART: u64 = 256_000_000;
pub const MAX_ENTRIES: usize = 10_000;
pub const MAX_RATIO: u64 = 100;

pub fn check_expansion(entries: usize, total: u64, size: u64, compressed: u64) -> Result<()> {
    if entries > MAX_ENTRIES {
        bail!("zip entry count exceeds 10000");
    }
    if total > MAX_TOTAL {
        bail!("zip total uncompressed size exceeds 512 MB");
    }
    if size > MAX_PART {
        bail!("zip part exceeds 256 MB");
    }
    if size > compressed.saturating_mul(MAX_RATIO) {
        bail!("zip compression ratio exceeds 100:1");
    }
    Ok(())
}
pub fn validate_zip(path: &std::path::Path) -> Result<zip::ZipArchive<File>> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let count = archive.len();
    let mut total = 0_u64;
    check_expansion(count, 0, 0, 0)?;
    for i in 0..archive.len() {
        let part = archive.by_index(i)?;
        total = total
            .checked_add(part.size())
            .context("zip size overflow")?;
        check_expansion(count, total, part.size(), part.compressed_size())?;
    }
    // Verify actual inflated bytes too: do not trust a forged central directory.
    let mut actual_total = 0_u64;
    for i in 0..archive.len() {
        let mut part = archive.by_index(i)?;
        let declared = part.size();
        let compressed = part.compressed_size();
        let actual = std::io::copy(&mut (&mut part).take(MAX_PART + 1), &mut std::io::sink())?;
        actual_total += actual;
        check_expansion(count, actual_total, actual, compressed)?;
        if actual != declared {
            bail!("zip declared and actual sizes differ");
        }
    }
    Ok(archive)
}
fn page(page: u32, text: String) -> Page {
    Page {
        page,
        text,
        needs_ocr: false,
    }
}
pub fn parse(request: &Request) -> Result<Vec<Page>> {
    parse_inner(request).map_err(crate::failures::parser)
}
fn parse_inner(request: &Request) -> Result<Vec<Page>> {
    let path = &request.path;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "txt" | "md" | "markdown" => Ok(vec![page(1, std::fs::read_to_string(path)?)]),
        "html" | "htm" => {
            let html = scraper::Html::parse_document(&std::fs::read_to_string(path)?);
            let selector = scraper::Selector::parse("h1,h2,h3,h4,h5,h6,p,li,tr,pre")
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut text = String::new();
            for node in html.select(&selector) {
                if let Some(level) = node
                    .value()
                    .name()
                    .strip_prefix('h')
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    text.push_str(&"#".repeat(level));
                    text.push(' ');
                }
                text.push_str(&node.text().collect::<Vec<_>>().join(" "));
                text.push('\n');
            }
            if text.is_empty() {
                text = html.root_element().text().collect::<Vec<_>>().join(" ");
            }
            Ok(vec![page(1, text)])
        }
        "docx" => {
            let mut zip = validate_zip(path)?;
            let mut xml = String::new();
            zip.by_name("word/document.xml")?
                .take(MAX_PART)
                .read_to_string(&mut xml)?;
            let mut reader = XmlReader::from_str(&xml);
            let mut text = String::new();
            loop {
                match reader.read_event()? {
                    Event::Text(t) => text.push_str(&quick_xml::escape::unescape(&t.decode()?)?),
                    Event::End(e) if e.local_name().as_ref() == b"p" => text.push('\n'),
                    Event::Empty(e) if e.local_name().as_ref() == b"tab" => text.push(' '),
                    Event::Empty(e) if e.local_name().as_ref() == b"br" => text.push('\n'),
                    Event::Eof => break,
                    _ => (),
                }
            }
            text.push_str(&crate::office::ingest_graphics(
                &mut zip,
                "word/document.xml",
            ));
            Ok(vec![page(1, text)])
        }
        "xlsx" => {
            let mut zip = validate_zip(path)?;
            let parts = crate::office::ingest_sheet_parts(&mut zip).unwrap_or_default();
            let mut workbook = open_workbook_auto(path)?;
            let names = workbook.sheet_names().to_vec();
            let mut pages = Vec::new();
            for (i, name) in names.into_iter().enumerate() {
                let range = workbook.worksheet_range(&name)?;
                let mut text = format!("# {name}\n");
                for row in range.rows() {
                    text.push_str(
                        &row.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" | "),
                    );
                    text.push('\n');
                }
                if let Some(part) = parts.get(&name) {
                    text.push_str(&crate::office::ingest_graphics(&mut zip, part));
                }
                pages.push(page(i as u32 + 1, text));
            }
            Ok(pages)
        }
        "pptx" => {
            let mut zip = validate_zip(path)?;
            let names = pptx_slide_order(&mut zip)?;
            let mut pages = Vec::new();
            for (i, name) in names.iter().enumerate() {
                let mut xml = String::new();
                zip.by_name(name)?.take(MAX_PART).read_to_string(&mut xml)?;
                let mut reader = XmlReader::from_str(&xml);
                let mut text = String::new();
                let mut in_text = false;
                loop {
                    match reader.read_event()? {
                        Event::Start(e) if e.local_name().as_ref() == b"t" => in_text = true,
                        Event::Text(t) if in_text => {
                            text.push_str(&quick_xml::escape::unescape(&t.decode()?)?)
                        }
                        Event::End(e) if e.local_name().as_ref() == b"t" => in_text = false,
                        Event::End(e) if e.local_name().as_ref() == b"p" => text.push('\n'),
                        Event::Eof => break,
                        _ => (),
                    }
                }
                text.push_str(&crate::office::ingest_graphics(&mut zip, name));
                pages.push(page(i as u32 + 1, text));
            }
            Ok(pages)
        }
        "pdf" => pdf(request),
        _ => bail!("unsupported format: {ext}"),
    }
}
/// Resolve presentation order; older incomplete packages fall back to numeric part suffixes.
fn pptx_slide_order(zip: &mut zip::ZipArchive<File>) -> Result<Vec<String>> {
    use std::collections::BTreeMap;
    let mut names: Vec<_> = zip
        .file_names()
        .filter(|n| n.starts_with("ppt/slides/") && !n.contains("/_rels/") && n.ends_with(".xml"))
        .map(str::to_owned)
        .collect();
    names.sort_by_key(|n| {
        n.strip_prefix("ppt/slides/slide")
            .and_then(|n| n.strip_suffix(".xml"))
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    if !zip.file_names().any(|n| n == "ppt/presentation.xml")
        || !zip
            .file_names()
            .any(|n| n == "ppt/_rels/presentation.xml.rels")
    {
        return Ok(names);
    }
    let resolved = (|| -> Result<Vec<String>> {
        let mut xml = String::new();
        zip.by_name("ppt/_rels/presentation.xml.rels")?
            .take(MAX_PART)
            .read_to_string(&mut xml)?;
        let mut reader = quick_xml::NsReader::from_str(&xml);
        let mut rels = BTreeMap::new();
        loop {
            match reader.read_event()? {
                Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                    let attrs = e
                        .attributes()
                        .map(|a| {
                            let a = a?;
                            Ok((
                                String::from_utf8(a.key.as_ref().to_vec())?,
                                a.decode_and_unescape_value(reader.decoder())?.into_owned(),
                            ))
                        })
                        .collect::<Result<BTreeMap<_, _>>>()?;
                    if attrs.get("Type").is_some_and(|t| t.ends_with("/slide"))
                        && attrs.get("TargetMode").is_none_or(|m| m != "External")
                    {
                        if let (Some(id), Some(target)) = (attrs.get("Id"), attrs.get("Target")) {
                            let name = crate::office::resolve("ppt/presentation.xml", target)?;
                            if names.contains(&name) {
                                rels.insert(id.clone(), name);
                            }
                        }
                    }
                }
                Event::Eof => break,
                _ => (),
            }
        }
        xml.clear();
        zip.by_name("ppt/presentation.xml")?
            .take(MAX_PART)
            .read_to_string(&mut xml)?;
        let mut reader = quick_xml::NsReader::from_str(&xml);
        let mut ordered = Vec::new();
        let mut in_list = false;
        loop {
            match reader.read_event()? {
                Event::Start(e) if e.local_name().as_ref() == b"sldIdLst" => in_list = true,
                Event::End(e) if e.local_name().as_ref() == b"sldIdLst" => in_list = false,
                Event::Start(e) | Event::Empty(e)
                    if in_list && e.local_name().as_ref() == b"sldId" =>
                {
                    for a in e.attributes() {
                        let a = a?;
                        let (ns, local) = reader.resolve_attribute(a.key);
                        if local.as_ref() == b"id"
                            && matches!(ns, quick_xml::name::ResolveResult::Bound(n) if n.as_ref() == b"http://schemas.openxmlformats.org/officeDocument/2006/relationships")
                        {
                            let id = a.decode_and_unescape_value(reader.decoder())?;
                            ordered.push(
                                rels.get(id.as_ref())
                                    .context("presentation slide relationship missing")?
                                    .clone(),
                            );
                        }
                    }
                }
                Event::Eof => break,
                _ => (),
            }
        }
        Ok(ordered)
    })();
    Ok(resolved
        .ok()
        .filter(|list| !list.is_empty())
        .unwrap_or(names))
}
fn pdf(request: &Request) -> Result<Vec<Page>> {
    use pdfium_render::prelude::*;
    let library = request
        .pdfium
        .as_ref()
        .context("PDFium library path required (run scripts/fetch-assets.py)")
        .map_err(crate::failures::infrastructure)?;
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(library).map_err(|e| crate::failures::infrastructure(e.into()))?,
    );
    let document = pdfium.load_pdf_from_file(&request.path, None)?;
    let mut pages = Vec::new();
    // Index by page: PdfPagesIterator ends silently at a page that fails to load,
    // which would index a truncated document as complete.
    let count = document.pages().len();
    for i in 0..count {
        let pdf_page = document.pages().get(i).map_err(crate::failures::pdf_page)?;
        let text = pdf_page.text().map_err(crate::failures::pdf_page)?.all();
        if !text.trim().is_empty() {
            pages.push(page(i as u32 + 1, text));
            continue;
        }
        #[cfg(target_os = "macos")]
        {
            let image = pdf_page
                .render_with_config(&PdfRenderConfig::new().set_target_width(1600))
                .map_err(crate::failures::pdf_page)?
                .as_image();
            let mut png = std::io::Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png)?;
            let text = crate::Ocr::recognize(&AppleVision, png.get_ref())
                .map_err(|e| crate::failures::infrastructure(anyhow::Error::msg(e)))?;
            pages.push(page(i as u32 + 1, text));
        }
        #[cfg(not(target_os = "macos"))]
        pages.push(Page {
            page: i as u32 + 1,
            text: String::new(),
            needs_ocr: true,
        });
    }
    Ok(pages)
}
/// Apple's local Vision framework, invoked only within the sandboxed worker.
pub struct AppleVision;
impl crate::Ocr for AppleVision {
    fn recognize(&self, image: &[u8]) -> crate::ComponentResult<String> {
        #[cfg(target_os = "macos")]
        {
            use std::io::Write;
            let run = || -> Result<String> {
                let helper = std::env::current_exe()?.with_file_name("vz-vision-ocr");
                let mut child = std::process::Command::new(helper)
                    .env_clear()
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .context("start Apple Vision OCR helper")?;
                child.stdin.take().context("OCR stdin")?.write_all(image)?;
                let output = child.wait_with_output()?;
                if !output.status.success() {
                    bail!("Apple Vision OCR failed");
                }
                Ok(String::from_utf8(output.stdout)?)
            };
            run().map_err(|e| format!("{e:#}"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = image;
            Err("needs OCR".into())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_order_and_numeric_fallback() {
        use std::io::Write;
        for explicit in [false, true] {
            let file = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
            let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for n in 1..=12 {
                zip.start_file(format!("ppt/slides/slide{n}.xml"), opts)
                    .unwrap();
                write!(zip, "<sld><p><t>Slide {n}</t></p></sld>").unwrap();
            }
            let expected: Vec<_> = if explicit {
                (1..=12).rev().collect()
            } else {
                (1..=12).collect()
            };
            if explicit {
                zip.start_file("ppt/presentation.xml", opts).unwrap();
                write!(zip, "<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><p:sldIdLst>").unwrap();
                for n in &expected {
                    write!(zip, "<p:sldId id=\"{}\" r:id=\"r{n}\"/>", n + 255).unwrap();
                }
                write!(zip, "</p:sldIdLst></p:presentation>").unwrap();
                zip.start_file("ppt/_rels/presentation.xml.rels", opts)
                    .unwrap();
                write!(zip, "<Relationships>").unwrap();
                for n in 1..=12 {
                    write!(zip, "<Relationship Id=\"r{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"../ppt/slides/slide{n}.xml\"/>").unwrap();
                }
                write!(zip, "</Relationships>").unwrap();
            }
            zip.finish().unwrap();
            let pages = parse(&Request {
                path: file.path().to_owned(),
                pdfium: None,
                office: None,
            })
            .unwrap();
            assert_eq!(pages.len(), 12);
            for (i, (page, n)) in pages.iter().zip(&expected).enumerate() {
                assert_eq!(page.page, i as u32 + 1);
                assert_eq!(page.text.trim(), format!("Slide {n}"));
            }
            let mut archive = validate_zip(file.path()).unwrap();
            let names = pptx_slide_order(&mut archive).unwrap();
            assert_eq!(
                names,
                expected
                    .iter()
                    .map(|n| format!("ppt/slides/slide{n}.xml"))
                    .collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn pptx_resolution_errors_and_empty_lists_fall_back_to_numeric_order() {
        use std::io::Write;
        let p = "http://schemas.openxmlformats.org/presentationml/2006/main";
        let r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let rel = "http://schemas.openxmlformats.org/package/2006/relationships";
        for target in [Some("Slides/Slide2.xml"), Some("slides/%73lide2.xml"), None] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("fallback.pptx");
            let mut z = zip::ZipWriter::new(File::create(&path).unwrap());
            let list = if target.is_some() {
                r#"<p:sldId id="256" r:id="slide"/>"#
            } else {
                ""
            };
            let relation = target
                .map(|target| {
                    format!(r#"<Relationship Id="slide" Type="{r}/slide" Target="{target}"/>"#)
                })
                .unwrap_or_default();
            for (name, body) in [
                (
                    "ppt/presentation.xml".to_owned(),
                    format!(
                        r#"<p:presentation xmlns:p="{p}" xmlns:r="{r}"><p:sldIdLst>{list}</p:sldIdLst></p:presentation>"#
                    ),
                ),
                (
                    "ppt/_rels/presentation.xml.rels".into(),
                    format!(r#"<Relationships xmlns="{rel}">{relation}</Relationships>"#),
                ),
                (
                    "ppt/slides/slide10.xml".into(),
                    "<sld><p><t>Slide ten</t></p></sld>".into(),
                ),
                (
                    "ppt/slides/slide2.xml".into(),
                    "<sld><p><t>Slide two</t></p></sld>".into(),
                ),
            ] {
                z.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
            let pages = parse(&Request {
                path: path.clone(),
                pdfium: None,
                office: None,
            })
            .unwrap();
            assert_eq!(
                pages.iter().map(|p| p.text.trim()).collect::<Vec<_>>(),
                vec!["Slide two", "Slide ten"]
            );
            assert!(crate::office::rebuild(
                "fallback.pptx",
                &std::fs::read(path).unwrap(),
                &Default::default()
            )
            .is_err());
        }
    }
    #[test]
    fn ordinary_ingest_does_not_require_listing_sanitizer() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chart.docx");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:proofErr w:type="spellStart"/><w:r><w:t>Searchable ordinary document</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        zip.finish().unwrap();
        let input = std::fs::read(&path).unwrap();
        assert!(crate::office::rebuild("chart.docx", &input, &Default::default()).is_err());
        let pages = parse(&Request {
            path,
            pdfium: None,
            office: None,
        })
        .unwrap();
        assert!(pages[0].text.contains("Searchable ordinary document"));
    }
    #[test]
    fn entry_limit() {
        assert!(check_expansion(MAX_ENTRIES + 1, 0, 0, 0)
            .unwrap_err()
            .to_string()
            .contains("entry count"));
    }
    #[test]
    fn total_limit() {
        assert!(check_expansion(1, MAX_TOTAL + 1, 1, 1)
            .unwrap_err()
            .to_string()
            .contains("total"));
    }
    #[test]
    fn part_limit() {
        assert!(check_expansion(1, MAX_PART + 1, MAX_PART + 1, MAX_PART)
            .unwrap_err()
            .to_string()
            .contains("part"));
    }
    #[test]
    fn ratio_limit() {
        assert!(check_expansion(1, 101, 101, 1)
            .unwrap_err()
            .to_string()
            .contains("ratio"));
        assert!(check_expansion(1, 100, 100, 1).is_ok());
    }
    fn forged_archive(
        parts: usize,
        compressed_size: u64,
        declared_size: u32,
    ) -> tempfile::NamedTempFile {
        use std::io::Write;
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
        for i in 0..parts {
            zip.start_file(
                format!("part-{i}.xml"),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            std::io::copy(&mut std::io::repeat(b'a').take(compressed_size), &mut zip).unwrap();
        }
        zip.finish().unwrap();
        // Forge size declarations only. The compressed ranges and offsets remain
        // real, so this exercises preflight refusal without allocating a bomb.
        let mut bytes = std::fs::read(file.path()).unwrap();
        for i in 0..bytes.len().saturating_sub(46) {
            if &bytes[i..i + 4] == b"PK\x01\x02" {
                bytes[i + 24..i + 28].copy_from_slice(&declared_size.to_le_bytes());
            }
            if &bytes[i..i + 4] == b"PK\x03\x04" {
                bytes[i + 22..i + 26].copy_from_slice(&declared_size.to_le_bytes());
            }
        }
        file.reopen().unwrap().write_all(&bytes).unwrap();
        file
    }
    #[test]
    fn real_archive_entry_refusal() {
        let file = forged_archive(MAX_ENTRIES + 1, 0, 0);
        assert!(validate_zip(file.path())
            .err()
            .unwrap()
            .to_string()
            .contains("entry count"));
    }
    #[test]
    fn real_archive_part_refusal() {
        let file = forged_archive(1, 100, (MAX_PART + 1) as u32);
        assert!(validate_zip(file.path())
            .err()
            .unwrap()
            .to_string()
            .contains("part"));
    }
    #[test]
    fn real_archive_total_refusal() {
        let file = forged_archive(3, 2 * 1024 * 1024, 200 * 1024 * 1024);
        assert!(validate_zip(file.path())
            .err()
            .unwrap()
            .to_string()
            .contains("total"));
    }
    #[test]
    fn real_archive_ratio_refusal() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(&vec![b'a'; 100_000]).unwrap();
        zip.finish().unwrap();
        assert!(validate_zip(tmp.path())
            .err()
            .unwrap()
            .to_string()
            .contains("ratio"));
    }
}

#[cfg(feature = "office-feasibility")]
#[path = "office_bytes.rs"]
pub mod office_bytes;
