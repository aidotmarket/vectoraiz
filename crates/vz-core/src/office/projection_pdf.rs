//! PDFium text/object path only. The metadata guard never extracts sale text.
//! Layered, incremental and compressed-xref PDFs refuse until default-state and
//! bounded object-stream decoding can be proven. No active-state inference.
use super::*;
use lopdf::{Document, Object};
use pdfium_render::prelude::*;
use std::io::Read;
static PDFIUM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn extract(
    input: &[u8],
    library: Option<&std::path::Path>,
    out: &mut Projection,
) -> Result<()> {
    metadata(input, out)?;
    // PDFium's process-global initialization/destruction cannot overlap when
    // this API is exercised directly by concurrent tests. Workers do one job.
    let _guard = PDFIUM_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("PROJECTION_PDFIUM_UNAVAILABLE: prior PDFium failure"))?;
    let library = library.context(
        "PROJECTION_PDFIUM_UNAVAILABLE: PDFium library required; no search/OCR fallback",
    )?;
    let pdfium =
        Pdfium::new(Pdfium::bind_to_library(library).context("PROJECTION_PDFIUM_UNAVAILABLE")?);
    let document = pdfium
        .load_pdf_from_byte_slice(input, None)
        .context("PROJECTION_PDF_MALFORMED_OR_ENCRYPTED")?;
    let count = document.pages().len();
    ensure!(count > 0, "PROJECTION_PDF_MALFORMED: no pages");
    for i in 0..count {
        let at = format!("pdf/page[{}]", i + 1);
        let page = document
            .pages()
            .get(i)
            .with_context(|| format!("PROJECTION_PDF_PAGE_UNREADABLE at {at}"))?;
        let boundaries = page.boundaries();
        let page_bounds = boundaries
            .crop()
            .or_else(|_| boundaries.media())
            .with_context(|| {
                format!("PROJECTION_PDF_CLASSIFICATION_UNKNOWN at {at}: page geometry")
            })?
            .bounds;
        let text = page
            .text()
            .with_context(|| format!("PROJECTION_PDF_PAGE_UNREADABLE at {at}"))?;
        let chars = text.chars();
        ensure!(
            chars.len() <= MAX_VALUES,
            "PROJECTION_INVENTORY_LIMIT_EXCEEDED at {at}: {} / {MAX_VALUES}",
            chars.len()
        );
        let mut segments: Vec<(bool, bool, String)> = Vec::new();
        for j in 0..chars.len() {
            let character = chars.get(j)?;
            let value = character.unicode_char().with_context(|| {
                format!("PROJECTION_PDF_CLASSIFICATION_UNKNOWN at {at}/char[{j}]: Unicode")
            })?;
            // PDFium inserts CR/LF/space separators without an owning object.
            // They carry no visibility claim and remain in their adjacent flow.
            let (invisible, offpage) = if value.is_whitespace() {
                segments
                    .last()
                    .map(|s| (s.0, s.1))
                    .unwrap_or((false, false))
            } else {
                let mode = character.render_mode().with_context(|| {
                    format!("PROJECTION_PDF_CLASSIFICATION_UNKNOWN at {at}/char[{j}]: render mode")
                })?;
                ensure!(
                    mode != PdfPageTextRenderMode::Unknown,
                    "PROJECTION_PDF_CLASSIFICATION_UNKNOWN at {at}/char[{j}]: render mode"
                );
                let bounds = character.loose_bounds().with_context(|| {
                    format!("PROJECTION_PDF_CLASSIFICATION_UNKNOWN at {at}/char[{j}]: geometry")
                })?;
                let coordinates = [
                    bounds.left().value,
                    bounds.right().value,
                    bounds.top().value,
                    bounds.bottom().value,
                ];
                ensure!(
                    coordinates.iter().all(|n| n.is_finite()),
                    "PROJECTION_PDF_CLASSIFICATION_UNKNOWN at {at}/char[{j}]: nonfinite geometry"
                );
                (
                    mode == PdfPageTextRenderMode::Invisible,
                    bounds.left().value < page_bounds.left().value
                        || bounds.bottom().value < page_bounds.bottom().value
                        || bounds.right().value > page_bounds.right().value
                        || bounds.top().value > page_bounds.top().value,
                )
            };
            if let Some(last) = segments
                .last_mut()
                .filter(|s| s.0 == invisible && s.1 == offpage)
            {
                last.2.push(value);
            } else {
                segments.push((invisible, offpage, value.to_string()));
            }
        }
        if segments.iter().all(|s| s.2.trim().is_empty()) {
            out.drop_at("no-text-pages", &at)?;
            continue;
        }
        for (index, (invisible, offpage, value)) in segments.into_iter().enumerate() {
            let kind = format!(
                "PDF text{}{}",
                if invisible {
                    " (invisible render mode 3)"
                } else {
                    ""
                },
                if offpage { " (off-page)" } else { "" }
            );
            out.add(&format!("{at}/segment[{index}]"), &kind, value, false)?;
        }
    }
    ensure!(
        out.nodes.iter().any(|n| !n.text.trim().is_empty()),
        "PROJECTION_PDF_NEEDS_OCR: needs OCR; no sale projection available"
    );
    out.warnings.push(format!("PDF: {} omitted no-text pages; no rendering or OCR. Invisible and off-page text remain included content.", out.drops["no-text-pages"]));
    Ok(())
}

fn metadata(input: &[u8], out: &mut Projection) -> Result<()> {
    ensure!(
        input.len() as u64 <= PART_BYTES,
        "PROJECTION_PART_BYTES_EXCEEDED at pdf: {} / {PART_BYTES}",
        input.len()
    );
    ensure!(
        input.starts_with(b"%PDF-"),
        "PROJECTION_PDF_MALFORMED: header"
    );
    // Conservative lexical preflight, including escaped PDF names. This scans
    // streams/strings too: false positives refuse; they never admit ambiguity.
    // Prevent lopdf's automatic unbounded object/xref-stream decompression and
    // historical-revision merge before invoking it on this restricted profile.
    for name in input.split(|b| *b == b'/').skip(1) {
        let raw: Vec<_> = name
            .iter()
            .copied()
            .take_while(|b| !b.is_ascii_whitespace() && !b"()<>[]{}/%".contains(b))
            .collect();
        let mut decoded = Vec::new();
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == b'#' && i + 2 < raw.len() {
                let value = std::str::from_utf8(&raw[i + 1..i + 3])
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
                    .context("PROJECTION_PDF_MALFORMED: escaped name")?;
                decoded.push(value);
                i += 3;
            } else {
                decoded.push(raw[i]);
                i += 1;
            }
        }
        ensure!(
            !matches!(
                decoded.as_slice(),
                b"OCProperties" | b"OCG" | b"OCMD" | b"OC"
            ),
            "PROJECTION_PDF_OPTIONAL_CONTENT_STATE_UNPROVEN: layered PDF; exclude document"
        );
        ensure!(!matches!(decoded.as_slice(), b"Prev" | b"XRefStm" | b"ObjStm" | b"XRef"), "PROJECTION_PDF_STRUCTURE_UNSUPPORTED: incremental/object/xref stream; save a classic unlayered PDF or exclude document");
        ensure!(
            decoded != b"Encrypt",
            "PROJECTION_PDF_ENCRYPTED: exclude document"
        );
    }
    let marker = input
        .windows(9)
        .rposition(|s| s == b"startxref")
        .context("PROJECTION_PDF_MALFORMED: startxref")?;
    let offset: usize = std::str::from_utf8(&input[marker + 9..])?
        .split_whitespace()
        .next()
        .context("PDF xref offset")?
        .parse()?;
    ensure!(
        input.get(offset..offset.saturating_add(4)) == Some(b"xref"),
        "PROJECTION_PDF_STRUCTURE_UNSUPPORTED: classic xref required"
    );
    let doc = Document::load_mem(input).context("PROJECTION_PDF_MALFORMED: metadata graph")?;
    ensure!(!doc.is_encrypted(), "PROJECTION_PDF_ENCRYPTED");
    ensure!(
        doc.objects.len() <= 10_000,
        "PROJECTION_PART_COUNT_EXCEEDED at pdf: {} / 10000",
        doc.objects.len()
    );
    // lopdf skips malformed objects; require every live xref object instead.
    for (id, entry) in &doc.reference_table.entries {
        match entry {
            lopdf::xref::XrefEntry::Normal { generation, .. } => {
                ensure!(
                    doc.objects.contains_key(&(*id, *generation)),
                    "PROJECTION_PDF_MALFORMED at pdf/object[{id}]: missing object"
                );
            }
            lopdf::xref::XrefEntry::Compressed { .. } => {
                anyhow::bail!("PROJECTION_PDF_STRUCTURE_UNSUPPORTED: compressed object")
            }
            _ => (),
        }
    }
    out.expanded_bytes = input.len() as u64;
    out.parts = doc.objects.len();
    walk(
        &Object::Dictionary(doc.trailer.clone()),
        "pdf/trailer",
        0,
        out,
    )?;
    for (id, object) in &doc.objects {
        let at = format!("pdf/object[{}:{}]", id.0, id.1);
        walk(object, &at, 0, out)?;
        if let Object::Stream(stream) = object {
            let decoded = match stream.dict.get(b"Filter") {
                Err(_) => stream.content.len() as u64,
                Ok(Object::Name(name)) if name == b"FlateDecode" => {
                    ensure!(stream.dict.get(b"DecodeParms").is_err(), "PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: DecodeParms");
                    let mut decoder = flate2::read::ZlibDecoder::new(stream.content.as_slice()).take(PART_BYTES + 1);
                    std::io::copy(&mut decoder, &mut std::io::sink()).with_context(|| format!("PROJECTION_PDF_MALFORMED at {at}: FlateDecode"))?
                }
                _ => anyhow::bail!("PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: only raw/FlateDecode streams are bounded"),
            };
            ensure!(
                decoded <= PART_BYTES,
                "PROJECTION_PART_BYTES_EXCEEDED at {at}: {decoded} / {PART_BYTES}"
            );
            ensure!(
                decoded <= (stream.content.len() as u64).saturating_mul(100),
                "PROJECTION_EXPANSION_RATIO_EXCEEDED at {at}: {decoded} / {} (100:1)",
                stream.content.len()
            );
            out.expanded_bytes += decoded;
            ensure!(
                out.expanded_bytes <= DOCUMENT_BYTES,
                "PROJECTION_DOCUMENT_BYTES_EXCEEDED at {at}: {} / {DOCUMENT_BYTES}",
                out.expanded_bytes
            );
        }
    }
    Ok(())
}

fn walk(object: &Object, at: &str, depth: usize, out: &mut Projection) -> Result<()> {
    ensure!(
        depth <= 8,
        "PROJECTION_PDF_DEPTH_EXCEEDED at {at}: {depth} / 8"
    );
    let dict = match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&stream.dict),
        Object::Array(values) => {
            for (i, value) in values.iter().enumerate() {
                walk(value, &format!("{at}/item[{i}]"), depth + 1, out)?;
            }
            None
        }
        _ => None,
    };
    if let Some(dict) = dict {
        for (key, value) in dict.iter() {
            let category = match key.as_slice() {
                b"Info" | b"Metadata" => Some("properties"),
                b"Annots" => Some("annotations"),
                b"EmbeddedFiles" | b"EF" => Some("attachments"),
                b"A" | b"AA" | b"OpenAction" | b"JS" | b"JavaScript" => Some("actions"),
                b"AcroForm" => Some("hidden-form-data"),
                _ => None,
            };
            if let Some(category) = category {
                out.drop_at(category, &format!("{at}/{}", String::from_utf8_lossy(key)))?;
            }
            walk(value, at, depth + 1, out)?;
        }
    }
    Ok(())
}
