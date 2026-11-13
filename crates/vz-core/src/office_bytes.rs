//! Descriptor/byte-only parsers for the opt-in feasibility executable.
use super::*;
fn validate_bytes(bytes: &[u8]) -> Result<zip::ZipArchive<std::io::Cursor<&[u8]>>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let count = archive.len();
    let mut total = 0;
    check_expansion(count, 0, 0, 0)?;
    for i in 0..count {
        let mut part = archive.by_index(i)?;
        let declared = part.size();
        let compressed = part.compressed_size();
        let actual = std::io::copy(&mut (&mut part).take(MAX_PART + 1), &mut std::io::sink())?;
        total += actual;
        check_expansion(count, total, actual, compressed)?;
        anyhow::ensure!(actual == declared, "zip actual size differs");
    }
    Ok(archive)
}
pub fn parse(ext: &str, bytes: &[u8]) -> Result<Vec<Page>> {
    match ext {
        "txt" | "md" | "markdown" => Ok(vec![page(1, String::from_utf8(bytes.to_vec())?)]),
        "html" | "htm" => {
            let html = scraper::Html::parse_document(&String::from_utf8(bytes.to_vec())?);
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
            let mut zip = validate_bytes(bytes)?;
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
            validate_bytes(bytes)?;
            let mut workbook = calamine::open_workbook_auto_from_rs(std::io::Cursor::new(bytes))?;
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
        "pdf" => bail!("PDF must use preloaded PDFium with supplied bytes"),
        _ => bail!("unsupported format: {ext}"),
    }
}
