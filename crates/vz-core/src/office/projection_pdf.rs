//! PDFium text/object path only. The metadata guard never extracts sale text.
//! lopdf validates metadata and final-revision structure, including object/xref
//! streams. Optional-content state remains unproven. No active-state inference.
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
    let expected_pages = metadata(input, out)?;
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
    ensure!(usize::from(count) == expected_pages, "PROJECTION_PDF_PAGE_TREE_UNPROVEN: PDFium page count {count} differs from validated {expected_pages}");
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

pub(super) fn metadata(input: &[u8], out: &mut Projection) -> Result<usize> {
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
                if let Some(value) = std::str::from_utf8(&raw[i + 1..i + 3])
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
                {
                    decoded.push(value);
                    i += 3;
                } else {
                    // Binary stream data is not PDF name syntax. Let lopdf
                    // validate actual names rather than rejecting random '#'.
                    decoded.push(raw[i]);
                    i += 1;
                }
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
        // lopdf can automatically decode these in xref/object streams before
        // the object filter runs. This profile only admits raw/FlateDecode.
        ensure!(
            !matches!(decoded.as_slice(), b"LZWDecode" | b"ASCII85Decode"),
            "PROJECTION_PDF_STREAM_UNSUPPORTED: only raw/FlateDecode streams are bounded"
        );
        ensure!(
            decoded != b"Encrypt",
            "PROJECTION_PDF_ENCRYPTED: exclude document"
        );
    }
    bound_flate_payloads(input)?;
    // The metadata parser merges revisions; none of their strings are emitted.
    // PDFium alone supplies final-revision page text. Bound object streams in
    // the load filter before lopdf automatically expands them.
    let doc = lopdf::Reader {
        buffer: input,
        document: Document::new(),
        encryption_state: None,
        raw_objects: BTreeMap::new(),
    }
    .read(Some(guard_object_stream))
    .context("PROJECTION_PDF_MALFORMED: metadata graph")?;
    for object in doc.objects.values() {
        if let Ok(dict) = object.as_dict() {
            if let Ok(error) = dict.get(b"VZGuardFailure") {
                anyhow::bail!("{}", String::from_utf8_lossy(error.as_str()?));
            }
        }
    }
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
            lopdf::xref::XrefEntry::Compressed { container, index } => {
                let stream = doc.get_object((*container, 0))?.as_stream()?;
                ensure!(
                    stream.dict.has_type(b"ObjStm") && doc.objects.contains_key(&(*id, 0)),
                    "PROJECTION_PDF_MALFORMED at pdf/object[{id}]: missing compressed object"
                );
                // lopdf recovers partial object streams. Require the xref's
                // container/index to identify the same object in its header.
                let first = usize::try_from(stream.dict.get(b"First")?.as_i64()?)?;
                let header = std::str::from_utf8(
                    stream
                        .content
                        .get(..first)
                        .context("PROJECTION_PDF_MALFORMED: object stream header")?,
                )?;
                let entries = header
                    .split_whitespace()
                    .map(str::parse::<u32>)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                ensure!(
                    entries.len()
                        == usize::try_from(stream.dict.get(b"N")?.as_i64()?)?.saturating_mul(2)
                        && entries.get(usize::from(*index) * 2) == Some(id),
                    "PROJECTION_PDF_MALFORMED at pdf/object[{id}]: compressed xref index"
                );
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
        &doc,
        out,
    )?;
    for (id, object) in &doc.objects {
        let at = format!("pdf/object[{}:{}]", id.0, id.1);
        walk(object, &at, 0, &doc, out)?;
        if let Object::Stream(stream) = object {
            let decoded = decoded_size(stream, &at)?;
            out.expanded_bytes += decoded;
            ensure!(
                out.expanded_bytes <= DOCUMENT_BYTES,
                "PROJECTION_DOCUMENT_BYTES_EXCEEDED at {at}: {} / {DOCUMENT_BYTES}",
                out.expanded_bytes
            );
        }
    }
    let pages = doc
        .catalog()
        .context("PROJECTION_PDF_MALFORMED: catalog")?
        .get(b"Pages")?
        .as_reference()?;
    let mut seen = BTreeSet::new();
    let count = page_tree(&doc, pages, 0, &mut seen)?;
    // All referenced streams have already passed bounded decode accounting.
    // Check content syntax independently: PDFium can recover malformed content
    // as an empty text page, which must not become a permitted omission.
    for id in seen {
        if doc.get_object(id)?.as_dict()?.get(b"Type")?.as_name()? == b"Page" {
            let content = doc.get_page_content(id).map_err(|_| {
                anyhow::anyhow!(
                    "PROJECTION_PDF_MALFORMED at pdf/object[{}:{}]: unreadable page content",
                    id.0,
                    id.1
                )
            })?;
            validate_content(&content, id)?;
        }
    }
    Ok(count)
}

// Xref streams are decoded inside Reader before its object callback. Preflight
// every possible zlib payload after a stream header, including old revisions,
// with a capped streaming decoder. This intentionally over-approximates PDF
// syntax: ambiguous matches may refuse, but cannot bypass allocation limits.
fn bound_flate_payloads(input: &[u8]) -> Result<()> {
    let mut total = input.len() as u64;
    let mut streams = 0;
    for (offset, word) in input.windows(6).enumerate() {
        if word != b"stream" {
            continue;
        }
        let mut start = offset + 6;
        while matches!(input.get(start), Some(b' ' | b'\t')) {
            start += 1;
        }
        match input.get(start) {
            Some(b'\r') => {
                start += 1;
                if input.get(start) == Some(&b'\n') {
                    start += 1;
                }
            }
            Some(b'\n') => start += 1,
            _ => continue,
        }
        let payload = &input[start..];
        // RFC 1950 CMF/FLG. Every FlateDecode stream accepted by lopdf's
        // ZlibDecoder has this header; raw streams require no expansion.
        if payload.len() < 2
            || payload[0] & 15 != 8
            || payload[0] >> 4 > 7
            || (u16::from(payload[0]) * 256 + u16::from(payload[1])) % 31 != 0
        {
            continue;
        }
        streams += 1;
        ensure!(
            streams <= 10_000,
            "PROJECTION_PART_COUNT_EXCEEDED: PDF streams"
        );
        let mut decoder = flate2::read::ZlibDecoder::new(payload);
        let decoded = std::io::copy(
            &mut (&mut decoder).take(PART_BYTES + 1),
            &mut std::io::sink(),
        )
        .context("PROJECTION_PDF_MALFORMED: FlateDecode preflight")?;
        ensure!(
            decoded <= PART_BYTES,
            "PROJECTION_PART_BYTES_EXCEEDED: PDF stream preflight"
        );
        ensure!(
            decoded <= decoder.total_in().saturating_mul(100),
            "PROJECTION_EXPANSION_RATIO_EXCEEDED: PDF stream preflight (100:1)"
        );
        total += decoded;
        ensure!(
            total <= DOCUMENT_BYTES,
            "PROJECTION_DOCUMENT_BYTES_EXCEEDED: PDF stream preflight"
        );
    }
    Ok(())
}

// This callback runs before ObjectStream::new(), which mutates the stream to
// decoded bytes. Preserve refusals as objects so the reader cannot hide
// them by silently skipping a malformed stream.
fn guard_object_stream(
    id: lopdf::ObjectId,
    object: &mut Object,
) -> Option<(lopdf::ObjectId, Object)> {
    if let Object::Stream(stream) = object {
        if stream.dict.has_type(b"ObjStm") {
            if let Err(error) = decoded_size(stream, "pdf/object-stream") {
                *object = Object::Dictionary(lopdf::dictionary! {
                    "VZGuardFailure" => Object::string_literal(error.to_string())
                });
            }
        }
    }
    Some((id, object.clone()))
}

fn decoded_size(stream: &lopdf::Stream, at: &str) -> Result<u64> {
    let decoded = match stream.dict.get(b"Filter") {
        Err(_) => stream.content.len() as u64,
        Ok(Object::Name(name)) if name == b"FlateDecode" => {
            // Predictor parameters are admitted for xref streams only, where
            // lopdf decodes them. The inflated bytes are bounded beforehand.
            if let Ok(params) = stream.dict.get(b"DecodeParms") {
                let params = params
                    .as_dict()
                    .context("PROJECTION_PDF_STREAM_UNSUPPORTED: DecodeParms")?;
                let number = |key: &[u8], default| -> Result<i64> {
                    Ok(match params.get(key) {
                        Ok(value) => value.as_i64()?,
                        Err(_) => default,
                    })
                };
                ensure!(
                    stream.dict.has_type(b"XRef")
                        && matches!(number(b"Predictor", 1)?, 1 | 10..=15)
                        && (1..=32).contains(&number(b"Columns", 1)?)
                        && number(b"Colors", 1)? == 1
                        && number(b"BitsPerComponent", 8)? == 8,
                    "PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: DecodeParms"
                );
            }
            let mut decoder =
                flate2::read::ZlibDecoder::new(stream.content.as_slice()).take(PART_BYTES + 1);
            std::io::copy(&mut decoder, &mut std::io::sink())
                .with_context(|| format!("PROJECTION_PDF_MALFORMED at {at}: FlateDecode"))?
        }
        _ => anyhow::bail!(
            "PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: only raw/FlateDecode streams are bounded"
        ),
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
    Ok(decoded)
}

fn validate_content(content: &[u8], id: lopdf::ObjectId) -> Result<()> {
    // The decoder accepts parsed prefixes. A private trailing sentinel proves
    // that malformed original bytes were not silently ignored.
    let mut bytes = content.to_vec();
    bytes.extend_from_slice(b"\nVZSaleProjectionEnd\n");
    let operations = lopdf::content::Content::decode(&bytes)
        .map_err(|_| {
            anyhow::anyhow!(
                "PROJECTION_PDF_MALFORMED at pdf/object[{}:{}]: invalid page content syntax",
                id.0,
                id.1
            )
        })?
        .operations;
    ensure!(
        operations
            .last()
            .is_some_and(|op| op.operator == "VZSaleProjectionEnd" && op.operands.is_empty()),
        "PROJECTION_PDF_MALFORMED at pdf/object[{}:{}]: incomplete page content",
        id.0,
        id.1
    );
    let mut text_open = false;
    for op in &operations[..operations.len() - 1] {
        match op.operator.as_str() {
            "BT" => {
                ensure!(
                    !text_open,
                    "PROJECTION_PDF_MALFORMED at pdf/object[{}:{}]: nested text object",
                    id.0,
                    id.1
                );
                text_open = true;
            }
            "ET" => {
                ensure!(
                    text_open,
                    "PROJECTION_PDF_MALFORMED at pdf/object[{}:{}]: unmatched text object",
                    id.0,
                    id.1
                );
                text_open = false;
            }
            _ => (),
        }
    }
    ensure!(
        !text_open,
        "PROJECTION_PDF_MALFORMED at pdf/object[{}:{}]: unclosed text object",
        id.0,
        id.1
    );
    Ok(())
}

fn page_tree(
    doc: &Document,
    id: lopdf::ObjectId,
    depth: usize,
    seen: &mut BTreeSet<lopdf::ObjectId>,
) -> Result<usize> {
    ensure!(
        depth <= 8 && seen.insert(id),
        "PROJECTION_PDF_PAGE_TREE_UNPROVEN at pdf/object[{}:{}]: depth/cycle/duplicate",
        id.0,
        id.1
    );
    let node = doc.get_object(id)?.as_dict()?;
    match node.get(b"Type")?.as_name()? {
        b"Page" => Ok(1),
        b"Pages" => {
            let mut count = 0;
            for child in node.get(b"Kids")?.as_array()? {
                count += page_tree(doc, child.as_reference()?, depth + 1, seen)?;
            }
            ensure!(node.get(b"Count")?.as_i64()? == count as i64, "PROJECTION_PDF_PAGE_TREE_UNPROVEN at pdf/object[{}:{}]: declared count differs from {count}", id.0, id.1);
            Ok(count)
        }
        _ => anyhow::bail!("PROJECTION_PDF_PAGE_TREE_UNPROVEN: unknown page node"),
    }
}

fn walk(
    object: &Object,
    at: &str,
    depth: usize,
    doc: &Document,
    out: &mut Projection,
) -> Result<()> {
    ensure!(
        depth <= 8,
        "PROJECTION_PDF_DEPTH_EXCEEDED at {at}: {depth} / 8"
    );
    let dict = match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&stream.dict),
        Object::Array(values) => {
            for (i, value) in values.iter().enumerate() {
                walk(value, &format!("{at}/item[{i}]"), depth + 1, doc, out)?;
            }
            None
        }
        Object::Name(name) => {
            ensure!(
                !matches!(name.as_slice(), b"OCG" | b"OCMD"),
                "PROJECTION_PDF_OPTIONAL_CONTENT_STATE_UNPROVEN: layered PDF; exclude document"
            );
            None
        }
        Object::Reference(id) => {
            ensure!(
                doc.objects.contains_key(id),
                "PROJECTION_PDF_MALFORMED at {at}: dangling object {}:{}",
                id.0,
                id.1
            );
            None
        }
        _ => None,
    };
    if let Some(dict) = dict {
        for (key, value) in dict.iter() {
            ensure!(
                !matches!(key.as_slice(), b"OCProperties" | b"OCG" | b"OCMD" | b"OC"),
                "PROJECTION_PDF_OPTIONAL_CONTENT_STATE_UNPROVEN: layered PDF; exclude document"
            );
            ensure!(
                key != b"Encrypt",
                "PROJECTION_PDF_ENCRYPTED: exclude document"
            );
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
            walk(value, at, depth + 1, doc, out)?;
        }
    }
    Ok(())
}
