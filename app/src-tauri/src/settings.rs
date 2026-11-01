//! Local answer selection. A resolved pins file is only a compatibility output.
use crate::models::Pin;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

#[derive(Clone, Debug)]
pub struct Settings(Value);
impl Settings {
    pub fn read(dir: &Path) -> Result<Self> {
        match fs::read(dir.join("settings.json")) {
            Ok(bytes) => {
                let value: Value = serde_json::from_slice(&bytes)?;
                ensure!(
                    value["version"] == 1 && value["answer"].is_object(),
                    "Unsupported settings file"
                );
                Ok(Self(value))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self(
                json!({"version":1,"answer":{"local_pin_sha256":null,"selection_initialised":false}}),
            )),
            Err(error) => Err(error.into()),
        }
    }
    pub fn selected(&self) -> Option<&str> {
        self.0["answer"]["local_pin_sha256"].as_str()
    }
    pub fn initialised(&self) -> bool {
        self.0["answer"]["selection_initialised"] == true
    }
    pub fn select(&mut self, sha256: Option<&str>) {
        self.0["answer"]["local_pin_sha256"] = json!(sha256);
        self.0["answer"]["selection_initialised"] = json!(true);
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        self.save_before_rename(dir, || Ok(()))
    }
    fn save_before_rename(
        &self,
        dir: &Path,
        before_rename: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let temp = dir.join(format!(".settings.json.{}", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&serde_json::to_vec(&self.0)?)?;
            file.sync_all()?;
            drop(file);
            before_rename()?;
            fs::rename(&temp, dir.join("settings.json"))?;
            Ok(())
        })();
        let _ = fs::remove_file(temp);
        result
    }
}

/// No hashing and no filesystem writes on a fresh install. Call before writing
/// models.selected.json; acquisitions initialise a fresh selection explicitly.
pub fn migrate(dir: &Path, embedding: &Pin, answers: &[Pin], ram_bytes: u64) -> Result<Settings> {
    let mut settings = Settings::read(dir)?;
    if settings.initialised() {
        return Ok(settings);
    }
    let installed = |pin: &Pin| {
        crate::models::regular(&dir.join(&pin.file), false)
            .and_then(|file| Ok(file.metadata()?.len() == pin.size))
            .unwrap_or(false)
    };
    let available: Vec<_> = answers.iter().filter(|pin| installed(pin)).collect();
    if available.is_empty() && !installed(embedding) {
        return Ok(settings);
    }
    let previous: Option<Value> = fs::read(dir.join("models.selected.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let selected = previous
        .as_ref()
        .and_then(|v| v["chat"]["sha256"].as_str())
        .and_then(|sha| available.iter().copied().find(|pin| pin.sha256 == sha))
        .or_else(|| {
            if available.len() == 1 {
                available.first().copied()
            } else {
                None
            }
        })
        .or_else(|| {
            // Only migration retains the old 0/16/32 GiB rule.
            let gib = ram_bytes.saturating_add(512 << 20) / (1 << 30);
            let old_index = if gib >= 32 {
                2
            } else if gib >= 16 {
                1
            } else {
                0
            };
            answers
                .get(old_index.min(answers.len().saturating_sub(1)))
                .filter(|pin| installed(pin))
        });
    settings.select(selected.map(|pin| pin.sha256.as_str()));
    settings
        .save(dir)
        .context("Save migrated answer selection")?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pin(file: &str) -> Pin {
        Pin {
            file: file.into(),
            size: 4,
            sha256: file.into(),
            metadata: json!({}),
        }
    }
    #[test]
    fn migration_table_and_turned_off_selection() {
        let search = pin("search");
        let answers = [pin("small"), pin("medium"), pin("large")];
        for (files, previous, expected) in [
            (vec![], None, None),
            (vec!["search"], None, None),
            (vec!["large"], None, Some("large")),
            (vec!["small", "medium", "large"], None, Some("large")),
            (
                vec!["small", "medium", "large"],
                Some("small"),
                Some("small"),
            ),
            (vec!["small", "large"], Some("unknown"), Some("large")),
        ] {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("models");
            if !files.is_empty() {
                fs::create_dir(&dir).unwrap();
            }
            for name in &files {
                fs::write(dir.join(name), b"gguf").unwrap();
            }
            if let Some(previous) = previous {
                fs::write(
                    dir.join("models.selected.json"),
                    json!({"chat":{"sha256":previous}}).to_string(),
                )
                .unwrap();
            }
            let result = migrate(&dir, &search, &answers, 48 << 30).unwrap();
            assert_eq!(result.selected(), expected);
            if files.is_empty() {
                assert!(!dir.exists());
                continue;
            }
            assert!(result.initialised());
            let mut result = result;
            result.select(None);
            result.save(&dir).unwrap();
            assert_eq!(
                migrate(&dir, &search, &answers, 48 << 30)
                    .unwrap()
                    .selected(),
                None
            );
        }
    }
    #[test]
    fn atomic_write_keeps_unknown_fields_and_previous_selection_on_interruption() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), json!({"version":1,"future":42,"answer":{"future":true,"local_pin_sha256":"old","selection_initialised":true}}).to_string()).unwrap();
        let mut settings = Settings::read(dir.path()).unwrap();
        settings.select(Some("new"));
        assert!(settings
            .save_before_rename(dir.path(), || anyhow::bail!("simulated kill before rename"))
            .is_err());
        assert_eq!(Settings::read(dir.path()).unwrap().selected(), Some("old"));
        settings.save(dir.path()).unwrap();
        let saved = Settings::read(dir.path()).unwrap();
        assert_eq!(saved.selected(), Some("new"));
        assert_eq!(saved.0["future"], 42);
        assert_eq!(saved.0["answer"]["future"], true);
    }
}
