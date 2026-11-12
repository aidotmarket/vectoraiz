//! Bounded, rebuilding handlers. Inventory is produced by a separate readback
//! of delivered bytes. Unsupported formats/types fail closed at any depth.
use anyhow::{ensure, Context, Result};
use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
    sync::Arc,
};

const MAX_DEPTH: usize = 8;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub location: String,
    pub kind: String,
    /// Null means unknown/absent, never an inferred zero or language.
    pub value: Option<String>,
    pub warning: bool,
}

/// Private inputs. Never serialize this object into delivered artifacts or a
/// handoff. Locations refer to the previous preview's actual delivered bytes.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Changes {
    #[serde(default)]
    pub edits: BTreeMap<String, String>,
    #[serde(default)]
    pub exclusions: BTreeSet<String>,
    #[serde(default)]
    pub identity_terms: Vec<String>,
    /// Required for Office or derived Office edits. This identifies the private
    /// immutable generation whose document/node values the seller reviewed.
    #[serde(default)]
    pub origin: Option<crate::review_store::CurrentReview>,
}

impl Changes {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.identity_terms.len() <= 128 && self.identity_terms.iter().all(|s| s.len() <= 256),
            "private identity input limit"
        );
        ensure!(
            self.edits.len() <= 50_000 && self.exclusions.len() <= 50_000,
            "private edit count limit"
        );
        let mut bytes = 0usize;
        for (key, value) in &self.edits {
            ensure!(
                key.len() <= 4096 && value.len() <= 1024 * 1024,
                "edit size limit"
            );
            bytes = bytes
                .checked_add(key.len() + value.len())
                .context("edit size overflow")?;
        }
        for key in &self.exclusions {
            ensure!(key.len() <= 4096, "exclusion size limit");
            bytes = bytes
                .checked_add(key.len())
                .context("exclusion size overflow")?;
        }
        ensure!(bytes <= 8 * 1024 * 1024, "total edit size limit");
        Ok(())
    }
}

/// Handler preview only: this memory result grants no generation, acceptance
/// or transfer authority. Native certification remains private to the builder.
pub struct ArtifactPreview {
    pub bytes: Vec<u8>,
    pub inventory: Vec<Entry>,
}
pub fn rebuild_artifact(name: &str, input: &[u8], changes: &Changes) -> Result<ArtifactPreview> {
    changes.validate()?;
    let bytes = rewrite(
        name,
        input,
        changes,
        &mut BTreeMap::new(),
        &mut Budget::default(),
    )?;
    let mut inventory = Vec::new();
    inspect(
        name,
        &bytes,
        changes,
        &mut inventory,
        &mut Budget::default(),
    )?;
    for (location, edit) in &changes.edits {
        ensure!(
            inventory
                .iter()
                .any(|entry| &entry.location == location && entry.value.as_ref() == Some(edit)),
            "edit did not reach delivered readback"
        );
    }
    Ok(ArtifactPreview { bytes, inventory })
}

#[derive(Default)]
pub(crate) struct Budget {
    bytes: usize,
    entries: usize,
    office: bool,
    zip_entries: usize,
    pub(crate) removals: Vec<vz_core::office::Removal>,
    pub(crate) office_warnings: Vec<String>,
    pub(crate) edit_bindings: BTreeMap<String, vz_core::office::Edit>,
    pub(crate) from_origin: BTreeSet<String>,
    pub(crate) chart_sources: BTreeMap<String, vz_core::office::ChartSources>,
    pub(crate) cancel: Option<tokio_util::sync::CancellationToken>,
    pub(crate) started: Option<std::time::Instant>,
    current_part: String,
    pub(crate) worker_time: BTreeMap<String, std::time::Duration>,
}
impl Budget {
    pub(crate) fn enable_office(&mut self) {
        self.office = true;
        self.started.get_or_insert_with(std::time::Instant::now);
    }
    fn max_bytes(&self) -> usize {
        if self.office {
            2 * 1024 * 1024 * 1024
        } else {
            MAX_BYTES
        }
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .context("expanded size overflow")?;
        self.entries += 1;
        if self.office {
            ensure!(
                self.bytes <= self.max_bytes(),
                "OFFICE_PACK_EXPANDED_BYTES_LIMIT_EXCEEDED at {}: {} / {} bytes",
                self.current_part,
                self.bytes,
                self.max_bytes()
            );
            ensure!(
                self.entries <= 20_000_000,
                "OFFICE_PACK_INVENTORY_LIMIT_EXCEEDED at {}: {} / 20000000 entries",
                self.current_part,
                self.entries
            );
        }
        ensure!(
            self.bytes <= self.max_bytes()
                && self.entries <= if self.office { 20_000_000 } else { MAX_ENTRIES },
            "inspection limit exceeded: {} bytes / {} entries",
            self.bytes,
            self.entries
        );
        Ok(())
    }
}

pub fn page<'a>(entries: &'a [Entry], search: &str, offset: usize, limit: usize) -> Vec<&'a Entry> {
    let search = search.to_lowercase();
    entries
        .iter()
        .filter(|e| {
            e.location.to_lowercase().contains(&search)
                || e.kind.to_lowercase().contains(&search)
                || e.value
                    .as_deref()
                    .unwrap_or("unknown/absent")
                    .to_lowercase()
                    .contains(&search)
        })
        .skip(offset)
        .take(limit.clamp(1, 200))
        .collect()
}

fn entry(
    out: &mut Vec<Entry>,
    location: &str,
    kind: &str,
    value: Option<String>,
    changes: &Changes,
    budget: &mut Budget,
) -> Result<()> {
    budget.current_part = location.into();
    budget.charge(location.len() + value.as_ref().map_or(0, String::len))?;
    let lower = value.as_deref().unwrap_or_default().to_lowercase();
    let category = format!("{kind} {location}").to_lowercase();
    let warning = ["confidential", "project", "client"]
        .iter()
        .any(|s| lower.contains(s) || category.contains(s))
        || changes
            .identity_terms
            .iter()
            .filter(|t| !t.trim().is_empty())
            .any(|t| lower.contains(&t.trim().to_lowercase()));
    out.push(Entry {
        location: location.into(),
        kind: kind.into(),
        value,
        warning,
    });
    Ok(())
}

/// Public/storage names and actual gateway aliases/host are blocking surfaces.
/// Call again at the destination boundary with the private current term set.
pub fn check_identifier(value: &str, terms: &[String]) -> Result<()> {
    let lower = value.to_lowercase();
    ensure!(
        !terms
            .iter()
            .filter(|t| !t.trim().is_empty())
            .any(|t| lower.contains(&t.trim().to_lowercase())),
        "identity match in public/storage identifier"
    );
    Ok(())
}

fn safe_path(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && !name.contains(['\\', ':', '\0'])
            && name
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."),
        "unsafe container path"
    );
    Ok(())
}

fn extension(name: &str) -> Result<&str> {
    let ext = name
        .rsplit_once('.')
        .context("format has no proven handler")?
        .1;
    ensure!(
        ["txt", "md", "markdown", "py", "json", "zip", "parquet", "docx", "xlsx", "pptx"]
            .contains(&ext),
        "format has no proven handler"
    );
    Ok(ext)
}

/// Rebuild one artifact in memory, bounded across all recursive calls. Mapping
/// is private and is intentionally a separate result from delivered bytes.
pub(crate) fn rewrite(
    name: &str,
    input: &[u8],
    changes: &Changes,
    mapping: &mut BTreeMap<String, String>,
    budget: &mut Budget,
) -> Result<Vec<u8>> {
    budget.current_part = name.into();
    transform(name, input, changes, mapping, budget, 0)
}

fn transform(
    name: &str,
    input: &[u8],
    changes: &Changes,
    mapping: &mut BTreeMap<String, String>,
    budget: &mut Budget,
    depth: usize,
) -> Result<Vec<u8>> {
    budget.current_part = name.into();
    if ["docx", "xlsx", "pptx"].contains(&extension(name)?) {
        budget.enable_office();
    }
    ensure!(depth <= MAX_DEPTH, "container depth limit");
    budget.charge(input.len())?;
    match extension(name)? {
        "docx" | "xlsx" | "pptx" => {
            let (bytes, report) = office_worker(name, input, budget, false)?;
            budget
                .chart_sources
                .insert(name.into(), report.chart_sources.clone());
            budget.zip_entries += report.source_parts;
            ensure!(
                budget.zip_entries <= 100_000,
                "OFFICE_PACK_ZIP_ENTRIES_LIMIT_EXCEEDED at {name}: {} / 100000 entries",
                budget.zip_entries
            );
            budget.charge(report.expanded_bytes as usize)?;
            let removal_warnings: BTreeSet<_> = report
                .removals
                .iter()
                .map(|r| format!("{}: {}", r.location, r.reason))
                .collect();
            budget.office_warnings.extend(
                report
                    .warnings
                    .into_iter()
                    .filter(|w| {
                        !removal_warnings.contains(w)
                            || ["chart", "SmartArt", "AlternateContent", "CELLRANGE"]
                                .iter()
                                .any(|kind| w.contains(kind))
                    })
                    .map(|w| format!("{name}!{w}")),
            );
            budget
                .removals
                .extend(report.removals.into_iter().map(|mut r| {
                    r.location = format!("{name}!{}", r.location);
                    r
                }));
            Ok(bytes)
        }
        "zip" => {
            let mut archive = zip::ZipArchive::new(Cursor::new(input))?;
            for i in 0..archive.len() {
                let part = archive.by_index(i)?;
                if [".docx", ".xlsx", ".pptx"]
                    .iter()
                    .any(|suffix| part.name().ends_with(suffix))
                {
                    budget.enable_office();
                }
            }
            budget.zip_entries += archive.len();
            ensure!(
                budget.zip_entries <= if budget.office { 100_000 } else { MAX_ENTRIES },
                "ZIP entry limit: {}",
                budget.zip_entries
            );
            let mut seen = BTreeSet::new();
            let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for i in 0..archive.len() {
                let mut part = archive.by_index(i)?; // encrypted entries refuse without a password
                let original = part.name().trim_end_matches('/');
                safe_path(original)?;
                ensure!(!original.contains('%'), "escaped ZIP paths are unsupported");
                ensure!(
                    seen.insert(original.to_lowercase()),
                    "duplicate/case-colliding ZIP path"
                );
                let mode = part.unix_mode().unwrap_or(0) & 0o170000;
                ensure!(
                    mode == 0 || mode == 0o100000 || (mode == 0o040000 && part.is_dir()),
                    "ZIP links/special entries forbidden"
                );
                if part.is_dir() {
                    continue;
                }
                let location = format!("{name}!{original}");
                if changes.exclusions.contains(&location) {
                    continue;
                }
                ensure!(
                    part.size() <= budget.max_bytes() as u64,
                    "ZIP expanded byte limit"
                );
                ensure!(
                    part.size() <= part.compressed_size().saturating_mul(100),
                    "ZIP expansion ratio exceeds 100:1 at {location}"
                );
                let declared = part.size();
                let ext = extension(original)?;
                let neutral = format!("document-{:06}.{ext}", i + 1);
                mapping.insert(location.clone(), format!("{name}!{neutral}"));
                let mut bytes = Vec::new();
                part.by_ref()
                    .take(budget.max_bytes() as u64 + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= budget.max_bytes(), "ZIP expanded byte limit");
                ensure!(
                    bytes.len() as u64 == declared,
                    "ZIP declared/actual size mismatch at {location}"
                );
                // Edits address neutral delivered locations, never source names.
                let bytes = transform(
                    &format!("{name}!{neutral}"),
                    &bytes,
                    changes,
                    mapping,
                    budget,
                    depth + 1,
                )?;
                output.start_file(
                    neutral,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )?;
                output.write_all(&bytes)?;
            }
            Ok(output.finish()?.into_inner())
        }
        "parquet" => rewrite_parquet(name, input, changes, budget),
        "json" => {
            let mut value: Value = serde_json::from_slice(input)?;
            rewrite_json(&mut value, name, changes, budget, depth)?;
            value.sort_all_objects();
            Ok(serde_json::to_vec_pretty(&value)?)
        }
        _ => {
            let text = std::str::from_utf8(input).context("unreadable text")?;
            ensure!(!text.contains('\0'), "binary text forbidden");
            ensure!(
                !input.starts_with(b"%PDF-")
                    && !input.starts_with(b"PK\x03\x04")
                    && !input.starts_with(b"PAR1"),
                "container disguised as text has no proven handler"
            );
            Ok(changes
                .edits
                .get(name)
                .map_or(text, String::as_str)
                .as_bytes()
                .to_vec())
        }
    }
}

fn rewrite_json(
    value: &mut Value,
    location: &str,
    changes: &Changes,
    budget: &mut Budget,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= 64, "JSON depth limit");
    budget.charge(0)?;
    match value {
        Value::Object(map) => {
            let old = std::mem::take(map);
            for (key, mut value) in old {
                let path = format!("{location}/{}", key.replace('~', "~0").replace('/', "~1"));
                if changes.exclusions.contains(&path) {
                    continue;
                }
                rewrite_json(&mut value, &path, changes, budget, depth + 1)?;
                let key = changes
                    .edits
                    .get(&format!("{path}/@key"))
                    .unwrap_or(&key)
                    .clone();
                ensure!(
                    map.insert(key, value).is_none(),
                    "duplicate rewritten JSON key"
                );
            }
        }
        Value::Array(values) => {
            for (i, value) in values.iter_mut().enumerate() {
                rewrite_json(
                    value,
                    &format!("{location}/{i}"),
                    changes,
                    budget,
                    depth + 1,
                )?;
            }
        }
        Value::String(text) => {
            if let Some(edit) = changes.edits.get(location) {
                *text = edit.clone();
            }
        }
        _ => ensure!(
            !changes.edits.contains_key(location),
            "only string values may be edited"
        ),
    }
    Ok(())
}

// Deliberately narrow Parquet type coverage. No binary/extension/struct/map
// column is passed through without a proven recursive rewrite handler.
fn supported(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Utf8
            | DataType::Boolean
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    ) || matches!(data_type, DataType::FixedSizeList(field, _) if field.data_type() == &DataType::Float32 && field.name() == "item" && field.metadata().is_empty())
}

// Native rows use content-bound identities so exclusions cannot retarget edits.
// Chunk ids alone are ordinal within a document and change meaning on rechunking.
fn row_location(name: &str, batch: &RecordBatch, row: usize, offset: usize) -> Result<String> {
    let identity = match name {
        "documents.parquet" => Some(("doc_id", false)),
        "chunks.parquet" => Some(("chunk_id", true)),
        _ => None,
    };
    if let Some((column, chunk)) = identity {
        let strings = |column: &str| -> Result<&StringArray> {
            batch
                .column_by_name(column)
                .context("missing row identity")?
                .as_any()
                .downcast_ref::<StringArray>()
                .context("invalid row identity")
        };
        let id = strings(column)?.value(row);
        let target = if chunk {
            serde_json::to_vec(&(id, strings("text")?.value(row)))?
        } else {
            serde_json::to_vec(&id)?
        };
        Ok(format!("{name}/id/{}", vz_core::pipeline::sha256(&target)))
    } else {
        Ok(format!("{name}/row/{}", offset + row))
    }
}

// These fields are consumed by the format, query example, citation resolution
// or native row identity. Descriptive columns may still be renamed.
pub const PROTECTED_COLUMNS: &[&str] = &[
    "embedding",
    "doc_id",
    "chunk_id",
    "text",
    "page",
    "pages",
    "file_name",
    "sha256",
    "type",
    "bytes",
    "token_count",
    "heading_path",
];

fn structural_column(name: &str) -> bool {
    PROTECTED_COLUMNS.contains(&name)
}

fn rewrite_parquet(
    name: &str,
    input: &[u8],
    changes: &Changes,
    budget: &mut Budget,
) -> Result<Vec<u8>> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::copy_from_slice(input))?;
    let schema = reader.schema();
    ensure!(
        schema.fields().iter().all(|f| supported(f.data_type())),
        "Parquet type has no proven handler"
    );
    let fields = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let key = format!("{name}/column/{i}/name");
            let renamed = changes.edits.get(&key).unwrap_or(field.name());
            ensure!(
                !structural_column(field.name()) || renamed == field.name(),
                "required format column cannot be renamed"
            );
            ensure!(
                renamed == field.name() || !structural_column(renamed),
                "required format column name is reserved"
            );
            Ok(Field::new(
                renamed,
                field.data_type().clone(),
                field.is_nullable(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut names = BTreeSet::new();
    ensure!(
        fields.iter().all(|f| names.insert(f.name())),
        "duplicate rewritten column"
    );
    let schema = Arc::new(Schema::new(fields)); // strips untrusted schema properties
    let mut writer = ArrowWriter::try_new(Vec::new(), schema.clone(), None)?;
    let mut row_offset = 0;
    for batch in reader.with_batch_size(256).build()? {
        let batch = batch?;
        budget.charge(batch.get_array_memory_size())?;
        let mut columns: Vec<ArrayRef> = Vec::new();
        for (column, array) in batch.columns().iter().enumerate() {
            if let Some(strings) = array.as_any().downcast_ref::<StringArray>() {
                let values = strings
                    .iter()
                    .enumerate()
                    .map(|(row, value)| {
                        let key = format!(
                            "{}/column/{column}",
                            row_location(name, &batch, row, row_offset)?
                        );
                        Ok(changes.edits.get(&key).map(String::as_str).or(value))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if batch.schema().field(column).name() == "heading_path" {
                    for text in values.iter().flatten() {
                        serde_json::from_str::<Vec<String>>(text)
                            .context("heading path must be a JSON string array")?;
                    }
                }
                columns.push(Arc::new(StringArray::from(values)));
            } else {
                columns.push(array.clone());
            }
        }
        // Generic edits to chunk text cannot preserve old vectors. The native
        // builder handles re-chunking/re-embedding before this handler runs.
        if batch.schema().index_of("embedding").is_ok() {
            ensure!(
                columns
                    .iter()
                    .zip(batch.columns())
                    .enumerate()
                    .all(
                        |(i, (a, b))| batch.schema().field(i).name() == "heading_path"
                            || a.to_data() == b.to_data()
                    ),
                "edit original text and rebuild embeddings"
            );
        }
        row_offset += batch.num_rows();
        writer.write(&RecordBatch::try_new(schema.clone(), columns)?)?;
    }
    Ok(writer.into_inner()?)
}

pub(crate) fn inspect(
    name: &str,
    input: &[u8],
    changes: &Changes,
    out: &mut Vec<Entry>,
    budget: &mut Budget,
) -> Result<()> {
    inspect_at(name, input, changes, out, budget, 0)
}

fn inspect_at(
    name: &str,
    input: &[u8],
    changes: &Changes,
    out: &mut Vec<Entry>,
    budget: &mut Budget,
    depth: usize,
) -> Result<()> {
    budget.current_part = name.into();
    if ["docx", "xlsx", "pptx"].contains(&extension(name)?) {
        budget.enable_office();
    }
    if extension(name)? == "zip" {
        let mut archive = zip::ZipArchive::new(Cursor::new(input))?;
        for i in 0..archive.len() {
            let part = archive.by_index(i)?;
            if [".docx", ".xlsx", ".pptx"]
                .iter()
                .any(|ext| part.name().ends_with(ext))
            {
                budget.enable_office();
            }
        }
    }
    ensure!(depth <= MAX_DEPTH, "container depth limit");
    check_identifier(name, &changes.identity_terms)?;
    budget.charge(input.len())?;
    entry(
        out,
        name,
        "delivery path",
        Some(name.into()),
        changes,
        budget,
    )?;
    match extension(name)? {
        "docx" | "xlsx" | "pptx" => {
            let (_, report) = office_worker(name, input, budget, true)?;
            budget.zip_entries += report.parts;
            ensure!(
                budget.zip_entries <= 100_000,
                "OFFICE_PACK_ZIP_ENTRIES_LIMIT_EXCEEDED at {name}: {} / 100000 entries",
                budget.zip_entries
            );
            budget.charge(report.expanded_bytes as usize)?;
            for v in report.inventory {
                let location = format!("{name}!{}", v.location);
                entry(out, &location, &v.kind, v.value, changes, budget)?;
                if v.warning {
                    out.last_mut().context("missing Office entry")?.warning = true;
                }
            }
            // Decode and join only XML text runs. Formatting attributes between
            // two runs must not hide a term; mark its contributing locations.
            let mut joined = String::new();
            let mut spans = Vec::new();
            for (index, e) in out.iter().enumerate().filter(|(_, e)| {
                e.location.starts_with(&format!("{name}!")) && e.kind == "Office text"
            }) {
                let start = joined.len();
                joined.push_str(&e.value.as_deref().unwrap_or_default().to_lowercase());
                spans.push((index, start, joined.len()));
            }
            for term in changes.identity_terms.iter().map(String::as_str).chain([
                "confidential",
                "project",
                "client",
            ]) {
                let term = term.trim().to_lowercase();
                if term.is_empty() {
                    continue;
                }
                for (start, _) in joined.match_indices(&term) {
                    let end = start + term.len();
                    for (index, left, right) in &spans {
                        if *left < end && *right > start {
                            out[*index].warning = true;
                        }
                    }
                }
            }
        }
        "zip" => {
            let mut zip = zip::ZipArchive::new(Cursor::new(input))?;
            budget.zip_entries += zip.len();
            ensure!(
                budget.zip_entries <= if budget.office { 100_000 } else { MAX_ENTRIES },
                "ZIP readback entry limit at {name}: {}",
                budget.zip_entries
            );
            let mut seen = BTreeSet::new();
            entry(
                out,
                &format!("{name}/count"),
                "container entries",
                Some(zip.len().to_string()),
                changes,
                budget,
            )?;
            for i in 0..zip.len() {
                let mut part = zip.by_index(i)?;
                safe_path(part.name())?;
                ensure!(
                    !part.name().contains('%') && seen.insert(part.name().to_lowercase()),
                    "escaped/duplicate/case-colliding rebuilt ZIP path"
                );
                let mode = part.unix_mode().unwrap_or(0) & 0o170000;
                ensure!(
                    mode == 0 || mode == 0o100000,
                    "rebuilt ZIP link/special entry forbidden"
                );
                ensure!(
                    !part.is_dir() && part.size() <= budget.max_bytes() as u64,
                    "invalid rebuilt ZIP entry"
                );
                let location = format!("{name}!{}", part.name());
                let declared = part.size();
                ensure!(
                    declared <= part.compressed_size().saturating_mul(100),
                    "ZIP readback expansion ratio exceeds 100:1 at {location}"
                );
                let mut bytes = Vec::new();
                part.by_ref()
                    .take(budget.max_bytes() as u64 + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 == declared,
                    "ZIP readback declared/actual size mismatch at {location}"
                );
                inspect_at(&location, &bytes, changes, out, budget, depth + 1)?;
            }
        }
        "parquet" => {
            let reader =
                ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::copy_from_slice(input))?;
            for (i, field) in reader.schema().fields().iter().enumerate() {
                ensure!(supported(field.data_type()), "unsupported readback type");
                entry(
                    out,
                    &format!("{name}/column/{i}/name"),
                    "column",
                    Some(field.name().clone()),
                    changes,
                    budget,
                )?;
                entry(
                    out,
                    &format!("{name}/column/{i}/type"),
                    "column type",
                    Some(field.data_type().to_string()),
                    changes,
                    budget,
                )?;
            }
            let mut offset = 0;
            for batch in reader.with_batch_size(256).build()? {
                let batch = batch?;
                budget.charge(batch.get_array_memory_size())?;
                for (column, array) in batch.columns().iter().enumerate() {
                    for row in 0..batch.num_rows() {
                        let value = if array.is_null(row) {
                            None
                        } else {
                            Some(arrow_cast::display::array_value_to_string(array, row)?)
                        };
                        entry(
                            out,
                            &format!(
                                "{}/column/{column}",
                                row_location(name, &batch, row, offset)?
                            ),
                            batch.schema().field(column).name(),
                            value,
                            changes,
                            budget,
                        )?;
                    }
                }
                offset += batch.num_rows();
            }
            entry(
                out,
                &format!("{name}/count"),
                "rows",
                Some(offset.to_string()),
                changes,
                budget,
            )?;
        }
        "json" => inspect_json(
            &serde_json::from_slice(input)?,
            name,
            changes,
            out,
            budget,
            0,
        )?,
        _ => {
            let text = std::str::from_utf8(input)?;
            entry(out, name, "text", Some(text.into()), changes, budget)?;
            for (i, line) in text.lines().enumerate().filter(|(_, l)| l.starts_with('#')) {
                entry(
                    out,
                    &format!("{name}/line/{}", i + 1),
                    "title/heading",
                    Some(line.into()),
                    changes,
                    budget,
                )?;
            }
        }
    }
    for kind in [
        "unmeasured counts",
        "language",
        "date range",
        "document properties",
        "aliases",
    ] {
        entry(out, &format!("{name}/@{kind}"), kind, None, changes, budget)?;
    }
    Ok(())
}

fn inspect_json(
    value: &Value,
    path: &str,
    changes: &Changes,
    out: &mut Vec<Entry>,
    budget: &mut Budget,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= 64, "JSON depth limit");
    match value {
        Value::Object(map) => {
            entry(
                out,
                &format!("{path}/@count"),
                "properties count",
                Some(map.len().to_string()),
                changes,
                budget,
            )?;
            for (key, value) in map {
                let location = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                entry(
                    out,
                    &format!("{location}/@key"),
                    "property",
                    Some(key.clone()),
                    changes,
                    budget,
                )?;
                inspect_json(value, &location, changes, out, budget, depth + 1)?;
            }
        }
        Value::Array(values) => {
            entry(
                out,
                &format!("{path}/@count"),
                "array count",
                Some(values.len().to_string()),
                changes,
                budget,
            )?;
            for (i, value) in values.iter().enumerate() {
                inspect_json(
                    value,
                    &format!("{path}/{i}"),
                    changes,
                    out,
                    budget,
                    depth + 1,
                )?;
            }
        }
        _ => entry(
            out,
            path,
            "value",
            match value {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                _ => Some(value.to_string()),
            },
            changes,
            budget,
        )?,
    }
    Ok(())
}

pub(crate) fn office_worker(
    name: &str,
    input: &[u8],
    budget: &mut Budget,
    readback: bool,
) -> Result<(Vec<u8>, vz_core::office::Report)> {
    use crate::review_store::{private_directory, private_file};
    use std::time::{Duration, Instant};
    let started = budget.started.unwrap_or_else(Instant::now);
    let remaining = Duration::from_secs(300).saturating_sub(started.elapsed());
    let prior = budget.worker_time.get(name).copied().unwrap_or_default();
    let document_remaining = Duration::from_secs(120).saturating_sub(prior);
    ensure!(
        !document_remaining.is_zero(),
        "OFFICE_DOCUMENT_DEADLINE_EXCEEDED at {name}: 120 seconds"
    );
    let document_start = Instant::now();
    ensure!(
        !remaining.is_zero(),
        "OFFICE_PACK_DEADLINE_EXCEEDED at {name}: 300 seconds"
    );
    let root = std::env::temp_dir()
        .canonicalize()?
        .join(format!("vz-office-{}", uuid::Uuid::new_v4()));
    private_directory(&root)?;
    let result = (|| {
        let source = root.join("input.bin");
        let mut file = private_file(&source)?;
        file.write_all(input)?;
        drop(file);
        let prefix = format!("{name}!");
        let edits = budget
            .edit_bindings
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix(&prefix)
                    .map(|key| (key.to_owned(), v.clone()))
            })
            .collect();
        let request = vz_core::worker::Request {
            path: source,
            pdfium: None,
            office: Some(vz_core::worker::OfficeJob {
                name: name.into(),
                output: root.clone(),
                edits,
                readback,
                sanitized_input: budget.from_origin.contains(name),
                chart_sources: budget.chart_sources.get(name).cloned().unwrap_or_default(),
            }),
        };
        let worker = vz_core::worker::executable()?;
        vz_core::worker::run_cancellable(
            &worker,
            &request,
            remaining.min(document_remaining),
            || {
                ensure!(
                    started.elapsed() < Duration::from_secs(300),
                    "OFFICE_PACK_DEADLINE_EXCEEDED at {name}: 300 seconds"
                );
                ensure!(
                    document_start.elapsed() < document_remaining,
                    "OFFICE_DOCUMENT_DEADLINE_EXCEEDED at {name}: 120 seconds"
                );
                ensure!(
                    !budget.cancel.as_ref().is_some_and(|c| c.is_cancelled()),
                    "listing cancelled during Office worker"
                );
                Ok(())
            },
        )?;
        let report: vz_core::office::Report =
            serde_json::from_slice(&std::fs::read(root.join("report.json"))?)?;
        ensure!(
            report.allowlist_version == vz_core::office::ALLOWLIST_VERSION,
            "OFFICE_WORKER_VERSION_MISMATCH at {name}"
        );
        let bytes = if readback {
            Vec::new()
        } else {
            std::fs::read(root.join("output.bin"))?
        };
        Ok((bytes, report))
    })();
    let cleanup = std::fs::remove_dir_all(&root);
    budget
        .worker_time
        .insert(name.into(), prior + document_start.elapsed());
    if result.is_ok() {
        cleanup?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_format_columns_cannot_be_renamed_or_shadowed() {
        for column in [
            "embedding",
            "doc_id",
            "chunk_id",
            "page",
            "pages",
            "text",
            "file_name",
            "sha256",
            "type",
            "bytes",
            "token_count",
            "heading_path",
        ] {
            let schema = Arc::new(Schema::new(vec![Field::new(column, DataType::Utf8, false)]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(StringArray::from(vec!["value"]))],
            )
            .unwrap();
            let mut writer = ArrowWriter::try_new(Vec::new(), schema, None).unwrap();
            writer.write(&batch).unwrap();
            let bytes = writer.into_inner().unwrap();
            let mut changes = Changes::default();
            changes
                .edits
                .insert("x.parquet/column/0/name".into(), "renamed".into());
            assert!(
                rebuild_artifact("x.parquet", &bytes, &changes).is_err(),
                "{column}"
            );
        }
    }

    #[test]
    fn unhandled_nested_parquet_properties_block() {
        let child = Arc::new(Field::new("item", DataType::Float32, true).with_metadata(
            std::collections::HashMap::from([("private-author".into(), "fixture-identity".into())]),
        ));
        let values = Arc::new(arrow_array::Float32Array::from(vec![1., 0.]));
        let array =
            arrow_array::FixedSizeListArray::try_new(child.clone(), 2, values, None).unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "embedding",
            DataType::FixedSizeList(child, 2),
            false,
        )]));
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(array)]).unwrap();
        let mut writer = ArrowWriter::try_new(Vec::new(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        assert!(rebuild_artifact(
            "nested.parquet",
            &writer.into_inner().unwrap(),
            &Changes::default()
        )
        .is_err());
    }
    #[test]
    fn nested_parquet_columns_values_and_properties_are_rebuilt() {
        let schema = Arc::new(Schema::new_with_metadata(
            vec![Field::new("client-code", DataType::Utf8, true)],
            std::collections::HashMap::from([("private-author".into(), "private-input".into())]),
        ));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(StringArray::from(vec![
                Some("Confidential Project Orion"),
                None,
            ]))],
        )
        .unwrap();
        let mut writer = ArrowWriter::try_new(Vec::new(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .start_file("client.parquet", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&writer.into_inner().unwrap()).unwrap();
        let mut changes = Changes::default();
        changes.identity_terms.push("Orion".into());
        changes.edits.insert(
            "x.zip!document-000001.parquet/column/0/name".into(),
            "reviewed-column".into(),
        );
        let bytes = rewrite(
            "x.zip",
            &archive.finish().unwrap().into_inner(),
            &changes,
            &mut BTreeMap::new(),
            &mut Budget::default(),
        )
        .unwrap();
        let mut inventory = Vec::new();
        inspect(
            "x.zip",
            &bytes,
            &changes,
            &mut inventory,
            &mut Budget::default(),
        )
        .unwrap();
        assert!(inventory
            .iter()
            .any(|e| e.value.as_deref() == Some("reviewed-column")));
        assert!(inventory
            .iter()
            .any(|e| e.warning && e.location.ends_with("/row/0/column/0")));
        assert!(!inventory
            .iter()
            .any(|e| e.value.as_deref() == Some("private-input")));
        assert!(inventory
            .iter()
            .any(|e| e.value.is_none() && e.location.ends_with("/row/1/column/0")));
    }

    #[test]
    fn depth_expansion_links_and_encryption_refuse() {
        let mut bytes = b"content".to_vec();
        for i in 0..10 {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file(
                if i == 0 { "x.txt" } else { "x.zip" },
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&bytes).unwrap();
            bytes = zip.finish().unwrap().into_inner();
        }
        assert!(rewrite(
            "x.zip",
            &bytes,
            &Changes::default(),
            &mut BTreeMap::new(),
            &mut Budget::default()
        )
        .is_err());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.add_symlink(
            "link.txt",
            "target",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        assert!(rewrite(
            "x.zip",
            &zip.finish().unwrap().into_inner(),
            &Changes::default(),
            &mut BTreeMap::new(),
            &mut Budget::default()
        )
        .is_err());
        let mut budget = Budget {
            bytes: MAX_BYTES,
            entries: 0,
            ..Budget::default()
        };
        assert!(rewrite(
            "x.txt",
            b"x",
            &Changes::default(),
            &mut BTreeMap::new(),
            &mut budget
        )
        .is_err());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("x.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"content").unwrap();
        let mut bytes = zip.finish().unwrap().into_inner();
        bytes[6] |= 1;
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 8] |= 1;
        assert!(rewrite(
            "x.zip",
            &bytes,
            &Changes::default(),
            &mut BTreeMap::new(),
            &mut Budget::default()
        )
        .is_err());
    }
    #[test]
    fn nested_rebuild_and_readback() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "private-client.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"{"title":"Confidential project","language":null}"#)
            .unwrap();
        let input = zip.finish().unwrap().into_inner();
        let mut changes = Changes::default();
        changes.edits.insert(
            "originals.zip!document-000001.json/title".into(),
            "Reviewed title".into(),
        );
        let mut mapping = BTreeMap::new();
        let bytes = rewrite(
            "originals.zip",
            &input,
            &changes,
            &mut mapping,
            &mut Budget::default(),
        )
        .unwrap();
        assert!(!bytes.windows(14).any(|w| w == b"private-client"));
        assert_eq!(mapping.len(), 1);
        let mut inventory = Vec::new();
        inspect(
            "originals.zip",
            &bytes,
            &changes,
            &mut inventory,
            &mut Budget::default(),
        )
        .unwrap();
        assert!(inventory
            .iter()
            .any(|e| e.value.as_deref() == Some("Reviewed title")));
        assert!(inventory
            .iter()
            .any(|e| e.value.is_none() && e.location.ends_with("/language")));
        assert!(!page(&inventory, "Reviewed", 0, 200).is_empty());
    }
    #[test]
    fn unsafe_or_unsupported_is_never_certified() {
        for name in ["x.pdf", "x.docx", "x.xlsx", "x.bin"] {
            assert!(rewrite(
                name,
                b"data",
                &Changes::default(),
                &mut BTreeMap::new(),
                &mut Budget::default()
            )
            .is_err());
        }
        for name in ["../escape.txt", "/absolute.txt", "a\\b.txt", "C:x.txt"] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"text").unwrap();
            assert!(rewrite(
                "x.zip",
                &zip.finish().unwrap().into_inner(),
                &Changes::default(),
                &mut BTreeMap::new(),
                &mut Budget::default()
            )
            .is_err());
        }
        assert!(check_identifier("neutral-bucket", &["BUCKET".into()]).is_err());
    }
}
