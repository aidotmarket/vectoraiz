//! Closed vector delivery contract. Shared by build and certification, so a
//! previously accepted originals generation cannot survive an upgrade.
#[cfg(not(feature = "listing-originals"))]
use anyhow::{ensure, Context, Result};
#[cfg(not(feature = "listing-originals"))]
use std::{collections::BTreeSet, path::Path};
pub(crate) const FILES: [&str; 6] = [
    "manifest.json",
    "chunks.parquet",
    "documents.parquet",
    "README.md",
    "examples/query.py",
    "examples/query.json",
];
#[cfg(not(feature = "listing-originals"))]
pub(crate) const ATTESTATION: &str =
    "The seller attests: chunks, embeddings and metadata; no source files are delivered.";
#[cfg(not(feature = "listing-originals"))]
pub(crate) fn verify(root: &Path) -> Result<()> {
    let mut actual = BTreeSet::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        ensure!(
            !entry.file_type().is_symlink(),
            "vector payload contains a link; fresh review required"
        );
        if entry.file_type().is_dir() {
            ensure!(
                entry.path() == root || entry.path() == root.join("examples"),
                "unexpected vector payload directory"
            );
            continue;
        }
        ensure!(entry.file_type().is_file(), "vector payload special entry");
        actual.insert(
            entry
                .path()
                .strip_prefix(root)?
                .to_str()
                .context("UTF-8 payload path")?
                .replace('\\', "/"),
        );
    }
    ensure!(
        actual == FILES.map(String::from).into(),
        "vector payload names differ from allowlist; fresh vector-only review required"
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    ensure!(
        manifest["originals"] == false,
        "legacy originals generation is stale; fresh vector-only review required"
    );
    let hashes = manifest["files"].as_object().context("manifest files")?;
    ensure!(
        hashes.keys().cloned().collect::<BTreeSet<_>>()
            == FILES
                .into_iter()
                .filter(|n| *n != "manifest.json")
                .map(String::from)
                .collect(),
        "vector manifest hash set differs from payload"
    );
    crate::verify(root)?;
    Ok(())
}
