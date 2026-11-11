//! Adapters consume frozen payloads, with live guards retained by the caller.
use crate::{
    export::inventory,
    review_store::{CurrentReview, NativePackReview},
};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

pub(crate) fn check_input(
    guard: &NativePackReview<'_>,
    current: &CurrentReview,
    supplied: &Path,
    payload: &Path,
) -> Result<()> {
    guard.check_accepted(current)?;
    let expected = inventory(payload, &current.pack_id.to_string())?;
    let actual = inventory(supplied, &current.pack_id.to_string())?;
    ensure!(
        serde_json::to_value(expected)? == serde_json::to_value(actual)?,
        "supplied pack is not the reviewed payload"
    );
    guard.check_accepted(current)?;
    Ok(())
}

pub(crate) fn package(
    guard: &NativePackReview<'_>,
    current: &CurrentReview,
    payload: &Path,
    output: &Path,
) -> Result<Value> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent.canonicalize()?;
    let output = parent.join(
        output
            .file_name()
            .context("pack destination needs a name")?,
    );
    ensure!(!output.try_exists()?, "pack output already exists");
    let stage = parent.join(format!(".vz-{}.partial", uuid::Uuid::new_v4()));
    let files = inventory(payload, &current.pack_id.to_string())?;
    let intent = guard.intent(current, &format!("package:{}", output.display()))?;
    fs::create_dir(&stage)?;
    let result = (|| {
        for file in &files {
            guard.check_accepted(current)?;
            let bytes = fs::read(payload.join(&file.path))?;
            ensure!(
                bytes.len() as u64 == file.size && vz_core::pipeline::sha256(&bytes) == file.sha256,
                "reviewed payload changed before write"
            );
            let target = stage.join(&file.path);
            fs::create_dir_all(target.parent().context("file parent")?)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            for bytes in bytes.chunks(65536) {
                guard.check_policy()?;
                output.write_all(bytes)?;
            }
            output.sync_all()?;
        }
        check_input(guard, current, &stage, payload)?;
        #[cfg(unix)]
        for entry in walkdir::WalkDir::new(&stage).contents_first(true) {
            let entry = entry?;
            if entry.file_type().is_dir() {
                fs::File::open(entry.path())?.sync_all()?;
            }
        }
        guard.check_accepted(current)?;
        crate::export::publish_folder(&stage, &output)?;
        #[cfg(unix)]
        fs::File::open(&parent)?.sync_all()?;
        // Promotion may already have happened if readback or durable receipt
        // fails. The intent stays uncertain; completed output is never removed.
        check_input(guard, current, &output, payload)?;
        let manifest = crate::verify(&output)?;
        guard.completed(&intent, &files)?;
        Ok(manifest)
    })();
    if result.is_err() && stage.try_exists().unwrap_or(false) {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
