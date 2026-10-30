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
            Ok(vec![page(1, text)])
        }
        "xlsx" => {
            validate_zip(path)?;
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
                pages.push(page(i as u32 + 1, text));
            }
            Ok(pages)
        }
        "pdf" => pdf(request),
        _ => bail!("unsupported format: {ext}"),
    }
}
fn pdf(request: &Request) -> Result<Vec<Page>> {
    use pdfium_render::prelude::*;
    let library = request
        .pdfium
        .as_ref()
        .context("PDFium library path required (run scripts/fetch-assets.py)")?;
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(library).map_err(|e| crate::failures::infrastructure(e.into()))?,
    );
    let document = pdfium.load_pdf_from_file(&request.path, None)?;
    let mut pages = Vec::new();
    for (i, pdf_page) in document.pages().iter().enumerate() {
        let text = pdf_page.text()?.all();
        if !text.trim().is_empty() {
            pages.push(page(i as u32 + 1, text));
            continue;
        }
        #[cfg(target_os = "macos")]
        {
            let image = pdf_page
                .render_with_config(&PdfRenderConfig::new().set_target_width(1600))?
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
