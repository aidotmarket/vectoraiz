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
    let (doc, expected_pages, actual_pages) = metadata_document(input, out)?;
    let boxes = inherited_boxes(&doc)?;
    drop(doc);
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
        let actual_text = actual_pages.contains(&usize::from(i));
        let at = format!("pdf/page[{}]", i + 1);
        let page = document
            .pages()
            .get(i)
            .with_context(|| format!("PROJECTION_PDF_PAGE_UNREADABLE at {at}"))?;
        let page_bounds = boxes[usize::from(i)];
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
            let Some(value) = projected_character(character.unicode_char()) else {
                continue;
            };
            if value == '\u{fffd}' {
                out.drop_at("unmapped-glyphs", &format!("{at}/char[{j}]"))?;
            }
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
                    bounds.left().value < page_bounds[0]
                        || bounds.bottom().value < page_bounds[1]
                        || bounds.right().value > page_bounds[2]
                        || bounds.top().value > page_bounds[3],
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
                "PDF text{}{}{}",
                if invisible {
                    " (invisible render mode 3)"
                } else {
                    ""
                },
                if offpage { " (off-page)" } else { "" },
                if actual_text {
                    " (not-visible ActualText may substitute glyphs)"
                } else {
                    ""
                }
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

#[cfg(test)]
pub(super) fn metadata(input: &[u8], out: &mut Projection) -> Result<usize> {
    Ok(metadata_document(input, out)?.1)
}

fn metadata_document(
    input: &[u8],
    out: &mut Projection,
) -> Result<(Document, usize, BTreeSet<usize>)> {
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
            if raw[i] == b'#' && i + 3 <= raw.len() {
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
        ensure!(
            decoded != b"Encrypt",
            "PROJECTION_PDF_ENCRYPTED: exclude document"
        );
    }
    let (structural_expanded, structural_encoded) = preflight_structural_streams(input)?;
    // The metadata parser merges revisions; none of their strings are emitted.
    // PDFium alone supplies final-revision page text. Bound object streams in
    // the load filter before lopdf automatically expands them.
    STRUCTURAL_GUARD.with(|state| {
        *state.borrow_mut() = (
            structural_expanded,
            input.len() as u64,
            None,
            structural_encoded,
        )
    });
    let loaded = lopdf::Reader {
        buffer: input,
        document: Document::new(),
        encryption_state: None,
        raw_objects: BTreeMap::new(),
    }
    .read(Some(guard_object_stream));
    let guarded = STRUCTURAL_GUARD.with(|state| state.borrow_mut().2.take());
    if let Some(error) = guarded {
        anyhow::bail!("{error}");
    }
    let doc = loaded.context("PROJECTION_PDF_MALFORMED: metadata graph")?;
    ensure!(!doc.is_encrypted(), "PROJECTION_PDF_ENCRYPTED");
    ensure!(
        doc.objects.len() <= 10_000,
        "PROJECTION_PART_COUNT_EXCEEDED at pdf: {} / 10000",
        doc.objects.len()
    );
    // lopdf skips malformed objects; require every live xref object instead.
    let mut referenced = BTreeSet::new();
    for object in doc
        .objects
        .values()
        .chain(std::iter::once(&Object::Dictionary(doc.trailer.clone())))
    {
        referenced_objects(object, &mut referenced);
    }
    for (id, entry) in &doc.reference_table.entries {
        match entry {
            lopdf::xref::XrefEntry::Normal { generation, offset } => {
                // Quartz sometimes advertises an unused, undefined object at
                // offset zero. A reference to it must still refuse.
                if *offset == 0
                    && !doc.objects.keys().any(|key| key.0 == *id)
                    && !referenced.contains(id)
                {
                    continue;
                }
                let mut pos = *offset as usize;
                let number = pdf_token(input, &mut pos)
                    .and_then(|(a, b)| std::str::from_utf8(&input[a..b]).ok()?.parse::<u32>().ok());
                let actual_generation = pdf_token(input, &mut pos)
                    .and_then(|(a, b)| std::str::from_utf8(&input[a..b]).ok()?.parse::<u16>().ok());
                ensure!(number == Some(*id) && actual_generation == Some(*generation),
                    "PROJECTION_PDF_XREF_OBJECT_MISMATCH at pdf/object[{id}:{generation}]: xref offset {offset} points to a different object header");
                ensure!(
                    doc.objects.contains_key(&(*id, *generation)),
                    "PROJECTION_PDF_OBJECT_UNREADABLE at pdf/object[{id}]: live xref entry could not be parsed (unsupported object syntax or invalid offset)"
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
    out.expanded_bytes = input.len() as u64 + STRUCTURAL_GUARD.with(|state| state.borrow().0);
    out.parts = doc.objects.len();
    walk(
        &Object::Dictionary(doc.trailer.clone()),
        "pdf/trailer",
        0,
        &doc,
        out,
    )?;
    let decoded_ids = decoded_stream_ids(&doc)?;
    let mut expanded = STRUCTURAL_GUARD.with(|state| state.borrow().0);
    let mut encoded = STRUCTURAL_GUARD.with(|state| state.borrow().3);
    let mut receipts = BTreeMap::new();
    for (id, object) in &doc.objects {
        let at = format!("pdf/object[{}:{}]", id.0, id.1);
        walk(object, &at, 0, &doc, out)?;
        if let Object::Stream(stream) = object {
            let decoded = if decoded_ids.contains(id) {
                let size = decoded_size(stream, &at)?;
                receipts.insert(*id, size);
                if !stream.dict.has_type(b"ObjStm") {
                    expanded += size;
                    encoded += stream.content.len() as u64;
                }
                size
            } else {
                stream.content.len() as u64
            };
            out.expanded_bytes += decoded;
            ensure!(
                out.expanded_bytes <= DOCUMENT_BYTES,
                "PROJECTION_DOCUMENT_BYTES_EXCEEDED at {at}: {} / {DOCUMENT_BYTES}",
                out.expanded_bytes
            );
        }
    }
    ensure!(
        expanded <= encoded.saturating_mul(100),
        "PROJECTION_EXPANSION_RATIO_EXCEEDED: decoded PDF document (100:1)"
    );
    let pages = doc
        .catalog()
        .context("PROJECTION_PDF_MALFORMED: catalog")?
        .get(b"Pages")?
        .as_reference()?;
    let mut seen = BTreeSet::new();
    let count = page_tree(&doc, pages, 0, &mut seen)?;
    // Every call into lopdf's unbounded decompressor must have a receipt.
    let mut actual_pages = BTreeSet::new();
    let mut validation = FormValidation {
        receipts: &receipts,
        expanded_bytes: out.expanded_bytes,
        consumed: BTreeSet::new(),
        active: BTreeSet::new(),
        memo: BTreeMap::new(),
    };
    for (index, id) in doc.get_pages().values().enumerate() {
        let mut content = Vec::new();
        for stream_id in doc.get_page_contents(*id) {
            content.extend(guarded_content(&doc, stream_id, &mut validation)?);
        }
        let resources = inherited(&doc, *id, b"Resources")?
            .unwrap_or(Object::Dictionary(lopdf::Dictionary::new()));
        if validate_forms(&doc, &content, *id, &resources, 0, &mut validation)? {
            actual_pages.insert(index);
        }
    }
    out.expanded_bytes = validation.expanded_bytes;
    Ok((doc, count, actual_pages))
}

// PDFium uses U+0002 for a removed line-end hyphen: join the word.
// Other non-whitespace C0 controls have the same counted replacement as
// unmappable glyphs, rather than carrying raw controls into sale text.
fn projected_character(value: Option<char>) -> Option<char> {
    match value {
        Some('\u{0002}') => None,
        Some(c) if c < ' ' && !matches!(c, '\t' | '\r' | '\n') => Some('\u{fffd}'),
        value => Some(value.unwrap_or('\u{fffd}')),
    }
}

fn referenced_objects(object: &Object, referenced: &mut BTreeSet<u32>) {
    match object {
        Object::Reference(id) => {
            referenced.insert(id.0);
        }
        Object::Array(values) => {
            for value in values {
                referenced_objects(value, referenced);
            }
        }
        Object::Dictionary(dict) => {
            for (_, value) in dict.iter() {
                referenced_objects(value, referenced);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                referenced_objects(value, referenced);
            }
        }
        _ => (),
    }
}

fn decoded_size(stream: &lopdf::Stream, at: &str) -> Result<u64> {
    let filters = match stream.dict.get(b"Filter") {
        Err(_) | Ok(Object::Null) => Vec::new(),
        Ok(Object::Name(name)) => vec![name.as_slice()],
        Ok(Object::Array(values)) => values
            .iter()
            .map(Object::as_name)
            .collect::<std::result::Result<Vec<_>, _>>()?,
        _ => anyhow::bail!("PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: filter shape"),
    };
    ensure!(
        filters.len() <= 2,
        "PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: filter chain"
    );
    let mut encoded = stream.content.as_slice();
    let mut intermediate = Vec::new();
    for (index, filter) in filters.iter().enumerate() {
        match *filter {
            b"ASCII85Decode" if index == 0 => {
                // ASCII85 'z' expands one byte into four; bound the worst case
                // before calling the library's non-streaming decoder.
                ensure!(
                    (encoded.len() as u64).saturating_mul(4) <= PART_BYTES,
                    "PROJECTION_PART_BYTES_EXCEEDED at {at}: ASCII85"
                );
                let ascii = lopdf::Stream::new(
                    lopdf::dictionary! {"Filter" => "ASCII85Decode"},
                    encoded.to_vec(),
                );
                intermediate = ascii
                    .decompressed_content()
                    .with_context(|| format!("PROJECTION_PDF_MALFORMED at {at}: ASCII85Decode"))?;
                encoded = &intermediate;
            }
            b"FlateDecode" if index + 1 == filters.len() => {
                if let Ok(params) = stream.dict.get(b"DecodeParms") {
                    let params = match params {
                        Object::Null => None,
                        Object::Dictionary(d) => Some(d),
                        Object::Array(a) if a.len() == 1 => Some(
                            a[0].as_dict()
                                .context("PROJECTION_PDF_STREAM_UNSUPPORTED: DecodeParms")?,
                        ),
                        _ => {
                            anyhow::bail!("PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: DecodeParms")
                        }
                    };
                    if let Some(params) = params {
                        let number = |key: &[u8], default| -> Result<i64> {
                            Ok(match params.get(key) {
                                Ok(value) => value.as_i64()?,
                                Err(_) => default,
                            })
                        };
                        ensure!(
                            matches!(number(b"Predictor", 1)?, 1 | 10..=15)
                                && (1..=1_000_000).contains(&number(b"Columns", 1)?)
                                && (1..=4).contains(&number(b"Colors", 1)?)
                                && matches!(number(b"BitsPerComponent", 8)?, 1 | 2 | 4 | 8 | 16),
                            "PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: DecodeParms"
                        );
                    }
                }
                let mut decoder = flate2::read::ZlibDecoder::new(encoded).take(PART_BYTES + 1);
                let decoded = std::io::copy(&mut decoder, &mut std::io::sink())
                    .with_context(|| format!("PROJECTION_PDF_MALFORMED at {at}: FlateDecode"))?;
                ensure!(
                    decoded <= PART_BYTES,
                    "PROJECTION_PART_BYTES_EXCEEDED at {at}: {decoded} / {PART_BYTES}"
                );
                return Ok(decoded);
            }
            _ => anyhow::bail!(
                "PROJECTION_PDF_STREAM_UNSUPPORTED at {at}: bounded raw/ASCII85/FlateDecode only"
            ),
        }
    }
    let decoded = if filters.is_empty() {
        stream.content.len()
    } else {
        intermediate.len()
    } as u64;
    ensure!(
        decoded <= PART_BYTES,
        "PROJECTION_PART_BYTES_EXCEEDED at {at}: {decoded} / {PART_BYTES}"
    );
    Ok(decoded)
}

fn validate_content(content: &[u8], id: lopdf::ObjectId) -> Result<()> {
    // Inline image decoding is outside the supported text-only profile.
    // Tokenize to avoid mistaking BI inside strings/comments for an operator.
    let mut pos = 0;
    while let Some((a, b)) = pdf_token(content, &mut pos) {
        ensure!(
            &content[a..b] != b"BI",
            "PROJECTION_PDF_INLINE_IMAGE_UNSUPPORTED"
        );
    }
    // The decoder accepts parsed prefixes. A private trailing sentinel proves
    // that malformed original bytes were not silently ignored.
    // Avoid inserting a second blank line after a trailing comment: lopdf's
    // operation parser consumes the comment newline but not a following blank.
    let mut bytes = content.trim_ascii_end().to_vec();
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
        // Undefined references are null (ISO 32000). Required page/content
        // structure is validated separately before PDFium can recover it.
        Object::Reference(_) => None,
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
                let location = format!("{at}/{}", String::from_utf8_lossy(key));
                if key == b"Annots" {
                    if let Ok(items) =
                        resolve(doc, value).and_then(|v| v.as_array().map_err(Into::into))
                    {
                        for (i, item) in items.iter().enumerate() {
                            if !matches!(resolve(doc, item)?, Object::Null) {
                                out.drop_at(category, &format!("{location}/item[{i}]"))?;
                            }
                        }
                    }
                } else if key != b"A"
                    || resolve(doc, value)?.as_dict().is_ok_and(|action| {
                        action.get(b"S").is_ok_and(|s| matches!(s, Object::Name(_)))
                    })
                {
                    out.drop_at(category, &location)?;
                }
            }
            walk(value, at, depth + 1, doc, out)?;
        }
    }
    Ok(())
}

fn resolve<'a>(doc: &'a Document, value: &'a Object) -> Result<&'a Object> {
    let mut value = value;
    for _ in 0..=8 {
        match value {
            Object::Reference(id) => value = doc.objects.get(id).unwrap_or(&Object::Null),
            _ => return Ok(value),
        }
    }
    anyhow::bail!("PROJECTION_PDF_DEPTH_EXCEEDED: indirect reference cycle")
}

fn inherited(doc: &Document, mut id: lopdf::ObjectId, key: &[u8]) -> Result<Option<Object>> {
    let mut seen = BTreeSet::new();
    for _ in 0..=8 {
        ensure!(
            seen.insert(id),
            "PROJECTION_PDF_PAGE_TREE_UNPROVEN: parent cycle"
        );
        let dict = doc.get_object(id)?.as_dict()?;
        if let Ok(value) = dict.get(key) {
            let value = resolve(doc, value)?;
            if !matches!(value, Object::Null) {
                return Ok(Some(value.clone()));
            }
        }
        match dict.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => id = parent,
            Err(_) => return Ok(None),
        }
    }
    anyhow::bail!("PROJECTION_PDF_DEPTH_EXCEEDED: inherited page attribute")
}

fn inherited_boxes(doc: &Document) -> Result<Vec<[f32; 4]>> {
    doc.get_pages()
        .values()
        .map(|id| {
            let read_box = |key: &[u8]| -> Result<Option<[f32; 4]>> {
                let Some(value) = inherited(doc, *id, key)? else {
                    return Ok(None);
                };
                let values = value.as_array()?;
                ensure!(
                    values.len() == 4,
                    "PROJECTION_PDF_CLASSIFICATION_UNKNOWN: page box length"
                );
                let mut bounds = [0f32; 4];
                for (i, value) in values.iter().enumerate() {
                    bounds[i] = resolve(doc, value)?.as_float()?;
                }
                ensure!(
                    bounds.iter().all(|n| n.is_finite()),
                    "PROJECTION_PDF_CLASSIFICATION_UNKNOWN: invalid page box"
                );
                let bounds = [
                    bounds[0].min(bounds[2]),
                    bounds[1].min(bounds[3]),
                    bounds[0].max(bounds[2]),
                    bounds[1].max(bounds[3]),
                ];
                ensure!(
                    bounds[0] < bounds[2] && bounds[1] < bounds[3],
                    "PROJECTION_PDF_CLASSIFICATION_UNKNOWN: invalid page box"
                );
                Ok(Some(bounds))
            };
            let mut bounds = read_box(b"MediaBox")?
                .context("PROJECTION_PDF_CLASSIFICATION_UNKNOWN: inherited page geometry")?;
            if let Some(crop) = read_box(b"CropBox")? {
                bounds = [
                    bounds[0].max(crop[0]),
                    bounds[1].max(crop[1]),
                    bounds[2].min(crop[2]),
                    bounds[3].min(crop[3]),
                ];
            }
            Ok(bounds)
        })
        .collect()
}

fn decoded_stream_ids(doc: &Document) -> Result<BTreeSet<lopdf::ObjectId>> {
    fn collect(
        doc: &Document,
        object: &Object,
        depth: usize,
        seen: &mut BTreeSet<lopdf::ObjectId>,
        decoded: &mut BTreeSet<lopdf::ObjectId>,
    ) -> Result<()> {
        ensure!(depth <= 32, "PROJECTION_PDF_DEPTH_EXCEEDED: resource graph");
        match object {
            Object::Reference(id) => {
                if seen.insert(*id) {
                    let value = doc.objects.get(id).unwrap_or(&Object::Null);
                    if matches!(value, Object::Stream(_)) {
                        decoded.insert(*id);
                    }
                    collect(doc, value, depth + 1, seen, decoded)?;
                }
            }
            Object::Array(values) => {
                for value in values {
                    collect(doc, value, depth + 1, seen, decoded)?;
                }
            }
            Object::Dictionary(dict) => {
                for (key, value) in dict.iter() {
                    if key == b"Resources" {
                        collect_resources(doc, value, depth + 1, seen, decoded)?;
                    } else {
                        collect(doc, value, depth + 1, seen, decoded)?;
                    }
                }
            }
            Object::Stream(s) => collect(
                doc,
                &Object::Dictionary(s.dict.clone()),
                depth + 1,
                seen,
                decoded,
            )?,
            _ => (),
        }
        Ok(())
    }
    fn collect_resources(
        doc: &Document,
        object: &Object,
        depth: usize,
        seen: &mut BTreeSet<lopdf::ObjectId>,
        decoded: &mut BTreeSet<lopdf::ObjectId>,
    ) -> Result<()> {
        ensure!(depth <= 32, "PROJECTION_PDF_DEPTH_EXCEEDED: resources");
        for (category, map) in resolve(doc, object)?.as_dict()?.iter() {
            if category == b"XObject" {
                for (_, entry) in resolve(doc, map)?.as_dict()?.iter() {
                    let image = resolve(doc, entry)?.as_stream().is_ok_and(|s| {
                        s.dict
                            .get(b"Subtype")
                            .is_ok_and(|v| v.as_name().is_ok_and(|n| n == b"Image"))
                    });
                    // Only this role exempts images (and their masks). Do not
                    // mark them seen: another role may require decoding them.
                    if !image {
                        collect(doc, entry, depth + 1, seen, decoded)?;
                    }
                }
            } else {
                collect(doc, map, depth + 1, seen, decoded)?;
            }
        }
        Ok(())
    }
    let mut decoded = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for id in doc.get_pages().values() {
        if let Ok(contents) = doc.get_object(*id)?.as_dict()?.get(b"Contents") {
            collect(doc, contents, 0, &mut seen, &mut decoded)?;
        }
        if let Some(resources) = inherited(doc, *id, b"Resources")? {
            collect_resources(doc, &resources, 0, &mut seen, &mut decoded)?;
        }
    }
    Ok(decoded)
}

// Every unbounded lopdf decompression is behind the actual stream's size
// receipt. Charge repeated decoding too, including duplicate page Contents:
// bounded individual streams must not create an unbounded concatenation.
fn guarded_content(
    doc: &Document,
    mut id: lopdf::ObjectId,
    validation: &mut FormValidation<'_>,
) -> Result<Vec<u8>> {
    for _ in 0..=8 {
        match doc.objects.get(&id) {
            Some(Object::Reference(next)) => id = *next,
            Some(Object::Stream(stream)) => {
                let size = *validation
                    .receipts
                    .get(&id)
                    .context("PROJECTION_PDF_STREAM_UNACCOUNTED")?;
                ensure!(size <= PART_BYTES, "PROJECTION_PART_BYTES_EXCEEDED");
                if !validation.consumed.insert(id) {
                    validation.expanded_bytes = validation.expanded_bytes.saturating_add(size);
                }
                ensure!(
                    validation.expanded_bytes <= DOCUMENT_BYTES,
                    "PROJECTION_DOCUMENT_BYTES_EXCEEDED: repeated content decoding"
                );
                return stream
                    .get_plain_content()
                    .context("PROJECTION_PDF_MALFORMED: unreadable content");
            }
            None | Some(Object::Null) => return Ok(Vec::new()),
            _ => anyhow::bail!("PROJECTION_PDF_MALFORMED: content is not a stream"),
        }
    }
    anyhow::bail!("PROJECTION_PDF_DEPTH_EXCEEDED: content reference cycle")
}

struct FormValidation<'a> {
    receipts: &'a BTreeMap<lopdf::ObjectId, u64>,
    expanded_bytes: u64,
    consumed: BTreeSet<lopdf::ObjectId>,
    active: BTreeSet<lopdf::ObjectId>,
    memo: BTreeMap<(lopdf::ObjectId, String), (bool, usize)>,
}

fn validate_forms(
    doc: &Document,
    content: &[u8],
    id: lopdf::ObjectId,
    resources: &Object,
    depth: usize,
    validation: &mut FormValidation<'_>,
) -> Result<bool> {
    let key = (id, format!("{resources:?}"));
    if let Some((result, validated_depth)) = validation.memo.get(&key) {
        if depth <= *validated_depth {
            return Ok(*result);
        }
    }
    ensure!(
        depth <= 8 && validation.active.insert(id),
        "PROJECTION_PDF_FORM_UNPROVEN: Form depth/cycle"
    );
    validate_content(content, id)?;
    let operations = lopdf::content::Content::decode(content)?.operations;
    let mut actual_text = false;
    for op in &operations {
        if matches!(op.operator.as_str(), "BDC" | "DP") {
            if let Some(properties) = op.operands.get(1) {
                let properties = if let Object::Name(name) = properties {
                    resolve(doc, resources)?
                        .as_dict()?
                        .get(b"Properties")
                        .ok()
                        .and_then(|p| resolve(doc, p).ok())
                        .and_then(|p| p.as_dict().ok())
                        .and_then(|p| p.get(name).ok())
                        .unwrap_or(&Object::Null)
                } else {
                    properties
                };
                actual_text |= resolve(doc, properties)?
                    .as_dict()
                    .is_ok_and(|p| p.has(b"ActualText"));
            }
        }
        if op.operator != "Do" {
            continue;
        }
        let name = op
            .operands
            .first()
            .context("PROJECTION_PDF_FORM_UNPROVEN: missing Do operand")?
            .as_name()?;
        let dict = resolve(doc, resources)?.as_dict()?;
        let Ok(xobjects) = dict.get(b"XObject") else {
            continue;
        };
        let xobjects = resolve(doc, xobjects)?.as_dict()?;
        let Ok(object) = xobjects.get(name) else {
            continue;
        };
        let value = resolve(doc, object)?;
        if matches!(value, Object::Null) {
            continue;
        }
        let stream = value.as_stream()?;
        if stream
            .dict
            .get(b"Subtype")
            .is_ok_and(|v| v.as_name().is_ok_and(|n| n == b"Form"))
        {
            let form_id = object
                .as_reference()
                .context("PROJECTION_PDF_FORM_UNPROVEN: direct Form stream")?;
            let own = stream.dict.get(b"Resources").unwrap_or(resources);
            if let Some((result, validated_depth)) =
                validation.memo.get(&(form_id, format!("{own:?}")))
            {
                if depth < *validated_depth {
                    actual_text |= *result;
                    continue;
                }
            }
            actual_text |= validate_forms(
                doc,
                &guarded_content(doc, form_id, validation)?,
                form_id,
                own,
                depth + 1,
                validation,
            )
            .context("PROJECTION_PDF_FORM_UNPROVEN: invoked Form content")?;
        }
    }
    validation.active.remove(&id);
    validation.memo.insert(key, (actual_text, depth));
    Ok(actual_text)
}

// This tokenizer skips strings, hex strings and comments so the word `stream`
// in page text cannot trigger an inflation attempt. Stream dictionaries are
// parsed by lopdf in a stream-free envelope; no parser expansion occurs there.
fn pdf_token(input: &[u8], pos: &mut usize) -> Option<(usize, usize)> {
    while *pos < input.len() {
        if input[*pos].is_ascii_whitespace() {
            *pos += 1;
            continue;
        }
        if input[*pos] == b'%' {
            while *pos < input.len() && !matches!(input[*pos], b'\r' | b'\n') {
                *pos += 1;
            }
            continue;
        }
        break;
    }
    let start = *pos;
    let b = *input.get(start)?;
    *pos += 1;
    match b {
        b'(' => {
            let mut depth = 1;
            while *pos < input.len() && depth > 0 {
                match input[*pos] {
                    b'\\' => {
                        *pos = (*pos + 2).min(input.len());
                        continue;
                    }
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => (),
                }
                *pos += 1;
            }
        }
        b'<' if input.get(*pos) != Some(&b'<') => {
            while *pos < input.len() && input[*pos] != b'>' {
                *pos += 1;
            }
            *pos = (*pos + 1).min(input.len());
        }
        b'<' | b'>' if input.get(*pos) == Some(&b) => *pos += 1,
        b'[' | b']' | b'>' | b'{' | b'}' => (),
        _ => {
            while *pos < input.len()
                && !input[*pos].is_ascii_whitespace()
                && !b"()<>[]{}/%".contains(&input[*pos])
            {
                *pos += 1;
            }
        }
    }
    Some((start, *pos))
}

// Guard errors live outside the document. No input dictionary can forge or
// suppress them. lopdf's pinned default-features=false Reader is synchronous.
thread_local! {
    static STRUCTURAL_GUARD: std::cell::RefCell<(u64, u64, Option<String>, u64)> = const { std::cell::RefCell::new((0, 0, None, 0)) };
}

fn guard_object_stream(
    id: lopdf::ObjectId,
    object: &mut Object,
) -> Option<(lopdf::ObjectId, Object)> {
    if let Object::Stream(stream) = object {
        if stream.dict.has_type(b"ObjStm") {
            let result = decoded_size(stream, "pdf/object-stream").and_then(|size| {
                STRUCTURAL_GUARD.with(|state| {
                    let mut state = state.borrow_mut();
                    state.0 += size;
                    state.3 += stream.content.len() as u64;
                    ensure!(
                        state.0 <= DOCUMENT_BYTES,
                        "PROJECTION_DOCUMENT_BYTES_EXCEEDED: structural streams"
                    );
                    ensure!(
                        state.0 <= state.1.saturating_mul(100),
                        "PROJECTION_EXPANSION_RATIO_EXCEEDED: structural streams (100:1)"
                    );
                    Ok(())
                })
            });
            if let Err(error) = result {
                STRUCTURAL_GUARD.with(|state| state.borrow_mut().2 = Some(format!("{error:#}")));
                // Prevent Reader from expanding a refused object stream.
                *object = Object::Null;
            }
        }
    }
    Some((id, object.clone()))
}

fn parse_dictionary(input: &[u8], pos: &mut usize) -> Result<lopdf::Dictionary> {
    let (begin, end) =
        pdf_token(input, pos).context("PROJECTION_PDF_MALFORMED: missing dictionary")?;
    ensure!(
        &input[begin..end] == b"<<",
        "PROJECTION_PDF_MALFORMED: dictionary start"
    );
    let mut depth = 1usize;
    while depth > 0 {
        let (start, end) =
            pdf_token(input, pos).context("PROJECTION_PDF_MALFORMED: unterminated dictionary")?;
        match &input[start..end] {
            b"<<" => depth += 1,
            b">>" => depth -= 1,
            _ => (),
        }
        ensure!(
            depth <= 8 && end - begin <= 1_000_000,
            "PROJECTION_PDF_DEPTH_EXCEEDED: stream dictionary"
        );
    }
    let mut envelope = b"%PDF-1.4\n1 0 obj\n".to_vec();
    envelope.extend_from_slice(&input[begin..*pos]);
    let offset = envelope.len() + b"\nendobj\n".len();
    envelope.extend_from_slice(format!("\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    let parsed =
        Document::load_mem(&envelope).context("PROJECTION_PDF_MALFORMED: stream dictionary")?;
    Ok(parsed.get_object((1, 0))?.as_dict()?.clone())
}

fn preflight_structural_streams(input: &[u8]) -> Result<(u64, u64)> {
    // Follow actual final/previous/hybrid xref offsets. Scanning possible
    // `stream` words cannot prove bounds: strings can contain those words and
    // opaque image bytes can contain fake headers/lengths. Object streams are
    // checked by the callback immediately before Reader expands them.
    let marker = input
        .windows(9)
        .rposition(|w| w == b"startxref")
        .context("PROJECTION_PDF_MALFORMED: startxref")?;
    let mut pos = marker + 9;
    let (start, end) =
        pdf_token(input, &mut pos).context("PROJECTION_PDF_MALFORMED: xref offset")?;
    let offset: usize = std::str::from_utf8(&input[start..end])?
        .parse()
        .context("PROJECTION_PDF_MALFORMED: xref offset")?;
    let mut pending = vec![offset];
    let mut seen = BTreeSet::new();
    let mut expanded = 0;
    let mut encoded = 0;
    while let Some(offset) = pending.pop() {
        ensure!(
            offset < input.len(),
            "PROJECTION_PDF_MALFORMED: xref offset outside input"
        );
        ensure!(
            seen.len() < 10_000,
            "PROJECTION_PART_COUNT_EXCEEDED: xref revisions"
        );
        if !seen.insert(offset) {
            continue;
        }
        let mut pos = offset;
        let (start, end) =
            pdf_token(input, &mut pos).context("PROJECTION_PDF_MALFORMED: xref header")?;
        let dict = if &input[start..end] == b"xref" {
            loop {
                let (start, end) =
                    pdf_token(input, &mut pos).context("PROJECTION_PDF_MALFORMED: xref trailer")?;
                if &input[start..end] == b"trailer" {
                    break;
                }
            }
            parse_dictionary(input, &mut pos)?
        } else {
            // The first token is the object number; consume generation + obj.
            pdf_token(input, &mut pos)
                .context("PROJECTION_PDF_MALFORMED: xref object generation")?;
            let (start, end) =
                pdf_token(input, &mut pos).context("PROJECTION_PDF_MALFORMED: xref object")?;
            ensure!(
                &input[start..end] == b"obj",
                "PROJECTION_PDF_MALFORMED: xref object header"
            );
            let dict = parse_dictionary(input, &mut pos)?;
            ensure!(
                dict.has_type(b"XRef"),
                "PROJECTION_PDF_MALFORMED: expected xref stream"
            );
            let (start, end) =
                pdf_token(input, &mut pos).context("PROJECTION_PDF_MALFORMED: xref stream")?;
            ensure!(
                &input[start..end] == b"stream",
                "PROJECTION_PDF_MALFORMED: xref stream header"
            );
            while matches!(input.get(pos), Some(b' ' | b'\t')) {
                pos += 1;
            }
            if input.get(pos) == Some(&b'\r') {
                pos += 1;
            }
            if input.get(pos) == Some(&b'\n') {
                pos += 1;
            }
            // Xref stream Length is required to be direct (ISO 32000).
            let length = usize::try_from(dict.get(b"Length")?.as_i64()?)
                .context("PROJECTION_PDF_MALFORMED: xref stream length")?;
            let end = pos
                .checked_add(length)
                .filter(|end| *end <= input.len())
                .context("PROJECTION_PDF_MALFORMED: xref stream length")?;
            let stream = lopdf::Stream::new(dict.clone(), input[pos..end].to_vec());
            encoded += stream.content.len() as u64;
            expanded += decoded_size(&stream, "pdf/xref-preflight")?;
            ensure!(
                expanded <= DOCUMENT_BYTES,
                "PROJECTION_DOCUMENT_BYTES_EXCEEDED: xref streams"
            );
            ensure!(
                expanded <= (input.len() as u64).saturating_mul(100),
                "PROJECTION_EXPANSION_RATIO_EXCEEDED: xref streams (100:1)"
            );
            dict
        };
        for key in [b"Prev".as_slice(), b"XRefStm"] {
            if let Ok(value) = dict.get(key) {
                pending.push(
                    usize::try_from(value.as_i64()?)
                        .context("PROJECTION_PDF_MALFORMED: previous xref offset")?,
                );
            }
        }
    }
    Ok((expanded, encoded))
}

#[cfg(test)]
pub(super) fn test_resources(doc: &Document, page: lopdf::ObjectId) -> lopdf::Dictionary {
    inherited(doc, page, b"Resources")
        .unwrap()
        .unwrap()
        .as_dict()
        .unwrap()
        .clone()
}

#[cfg(test)]
mod character_tests {
    #[test]
    fn repeat_decode_budget_is_checked_before_allocating() {
        let mut doc = lopdf::Document::new();
        let stream = lopdf::Stream::new(lopdf::dictionary! {}, b"q Q".to_vec());
        let size = super::decoded_size(&stream, "test").unwrap();
        let id = doc.add_object(stream);
        let alias = doc.add_object(lopdf::Object::Reference(id));
        let receipts = [(id, size)].into_iter().collect();
        let mut validation = super::FormValidation {
            receipts: &receipts,
            expanded_bytes: super::DOCUMENT_BYTES,
            consumed: Default::default(),
            active: Default::default(),
            memo: Default::default(),
        };
        assert_eq!(
            super::guarded_content(&doc, alias, &mut validation).unwrap(),
            b"q Q"
        );
        let error = super::guarded_content(&doc, id, &mut validation).unwrap_err();
        assert!(error
            .to_string()
            .contains("PROJECTION_DOCUMENT_BYTES_EXCEEDED"));
        let mut unaccounted = super::FormValidation {
            receipts: &Default::default(),
            expanded_bytes: 0,
            consumed: Default::default(),
            active: Default::default(),
            memo: Default::default(),
        };
        assert!(super::guarded_content(&doc, alias, &mut unaccounted)
            .unwrap_err()
            .to_string()
            .contains("PROJECTION_PDF_STREAM_UNACCOUNTED"));
    }

    #[test]
    fn controls_are_counted_replacements_and_hyphen_marker_joins() {
        for code in 0..32 {
            let c = char::from(code);
            let expected = match c {
                '\u{0002}' => None,
                '\t' | '\r' | '\n' => Some(c),
                _ => Some('\u{fffd}'),
            };
            assert_eq!(super::projected_character(Some(c)), expected);
        }
        assert_eq!(super::projected_character(None), Some('\u{fffd}'));
    }
}
