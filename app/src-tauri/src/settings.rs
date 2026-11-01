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
pub struct Settings(pub Value);
impl Settings {
    fn with_paths(mut value: Value) -> Self {
        if !value["paths"].is_object() {
            let default = if value["answer"]["local_pin_sha256"].is_string() {
                json!("local")
            } else {
                Value::Null
            };
            value["paths"] = json!({"default":default,"api_key":{"enabled":false,"provider":null,"key_present":false,"key_last4":null,"consent":null,"pending":null,"cleanup":[]}});
        }
        Self(value)
    }
    pub fn default_path(&self) -> Option<&str> {
        self.0["paths"]["default"].as_str()
    }
    pub fn api(&self) -> &Value {
        &self.0["paths"]["api_key"]
    }
    pub fn api_mut(&mut self) -> &mut Value {
        &mut self.0["paths"]["api_key"]
    }

    pub fn read(dir: &Path) -> Result<Self> {
        match fs::read(dir.join("settings.json")) {
            Ok(bytes) => {
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                    anyhow::anyhow!(
                        "Settings file could not be read. Use Start over or delete settings.json."
                    )
                })?;
                ensure!(
                    value["version"] == 1 && value["answer"].is_object(),
                    "Unsupported settings file"
                );
                Ok(Self::with_paths(value))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::with_paths(
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
        if sha256.is_some() && self.0["paths"]["default"].is_null() {
            self.0["paths"]["default"] = json!("local");
        }
        if sha256.is_none() && self.default_path() == Some("local") {
            self.0["paths"]["default"] = Value::Null;
        }
        self.0["answer"]["local_pin_sha256"] = json!(sha256);
        self.0["answer"]["selection_initialised"] = json!(true);
    }
    fn save(&self, dir: &Path) -> Result<()> {
        self.save_before_rename(dir, || Ok(()))
    }
    fn save_before_rename(
        &self,
        dir: &Path,
        before_rename: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        fs::create_dir_all(dir)?;
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

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use tokio::sync::watch;
use vz_core::cloud_gate::{ConsentState, Provider};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Running,
    Cancelled,
    Revoked,
    Committed,
}
#[derive(Clone)]
pub struct Operation {
    pub kind: &'static str,
    pub epoch: u64,
    pub mark: Mark,
    pub cancel: watch::Sender<bool>,
}
#[derive(Clone)]
pub struct SettingsState {
    pub settings: Settings,
    pub cloud_epoch: u64,
    pub cloud_stopped: bool,
    pub stop_failed: bool,
    pub cleanup_memory: Vec<Provider>,
    pub ops: HashMap<String, Operation>,
}
impl SettingsState {
    pub fn consent_state(&self) -> ConsentState {
        let api = self.settings.api();
        ConsentState {
            enabled: api["enabled"] == true,
            stopped: self.cloud_stopped || self.stop_failed,
            provider: serde_json::from_value::<Provider>(api["provider"].clone()).ok(),
            consent: serde_json::from_value(api["consent"].clone()).ok(),
            key_present: api["key_present"] == true,
        }
    }
}
fn states() -> &'static Mutex<HashMap<PathBuf, SettingsState>> {
    static STATES: OnceLock<Mutex<HashMap<PathBuf, SettingsState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}
/// Memory-only mutation, under the same mutex as update. Never await here.
pub fn with_state<T>(dir: &Path, f: impl FnOnce(&mut SettingsState) -> T) -> Result<T, String> {
    let mut states = states().lock().unwrap();
    if !states.contains_key(dir) {
        let settings = Settings::read(dir).map_err(|e| e.to_string())?;
        states.insert(
            dir.to_owned(),
            SettingsState {
                settings,
                cloud_epoch: 0,
                cloud_stopped: false,
                stop_failed: false,
                cleanup_memory: vec![],
                ops: HashMap::new(),
            },
        );
    }
    Ok(f(states.get_mut(dir).unwrap()))
}
pub fn read(dir: &Path) -> Result<Settings, String> {
    with_state(dir, |s| s.settings.clone())
}
/// Copy the WHOLE state, atomically persist settings, and publish while still
/// holding the mutex. Both closure and I/O failures publish nothing.

#[derive(Debug)]
pub enum UpdateError<E> {
    Closure(E),
    Read(String),
    Write(String),
}
impl<E: std::fmt::Display> std::fmt::Display for UpdateError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closure(e) => e.fmt(f),
            Self::Read(e) | Self::Write(e) => e.fmt(f),
        }
    }
}
impl<E: std::fmt::Display> From<UpdateError<E>> for String {
    fn from(error: UpdateError<E>) -> Self {
        error.to_string()
    }
}
pub fn update<E>(
    dir: &Path,
    f: impl FnOnce(&mut SettingsState) -> Result<(), E>,
) -> Result<Settings, UpdateError<E>> {
    with_state(dir, |state| {
        let mut next=state.clone();
        f(&mut next).map_err(UpdateError::Closure)?;
        next.settings.save(dir).map_err(|_|UpdateError::Write(format!("vectorAIz can't save settings in {}. Move the model folder to a place you can write to.",dir.display())))?;
        let settings=next.settings.clone(); *state=next; Ok(settings)
    }).map_err(UpdateError::Read)?
}

#[cfg(test)]
pub fn restart(dir: &Path) {
    states().lock().unwrap().remove(dir);
}

/// No hashing and no filesystem writes on a fresh install. Call before writing
/// models.selected.json; acquisitions initialise a fresh selection explicitly.
pub fn migrate(dir: &Path, embedding: &Pin, answers: &[Pin], ram_bytes: u64) -> Result<Settings> {
    let settings = read(dir).map_err(anyhow::Error::msg)?;
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
    let selected = selected.map(|pin| pin.sha256.clone());
    update(dir, |state| {
        state.settings.select(selected.as_deref());
        Ok::<_, String>(())
    })
    .map_err(anyhow::Error::msg)
    .context("Save migrated answer selection")
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
    fn corrupt_settings_has_plain_recovery_message() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), b"{broken").unwrap();
        assert_eq!(
            Settings::read(dir.path()).unwrap_err().to_string(),
            "Settings file could not be read. Use Start over or delete settings.json."
        );
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
            restart(&dir);
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

#[cfg(test)]
mod serialization_tests {
    use super::*;
    #[test]
    fn whole_state_rollback_and_local_activation_stop_interleavings() {
        for stop_first in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            update(dir.path(), |s| {
                s.settings.api_mut()["enabled"] = json!(true);
                Ok::<(), String>(())
            })
            .unwrap();
            let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
            let release = std::sync::Arc::new(std::sync::Barrier::new(2));
            let p = dir.path().to_owned();
            let e = entered.clone();
            let r = release.clone();
            let first = std::thread::spawn(move || {
                update(&p, |s| {
                    if stop_first {
                        s.settings.api_mut()["enabled"] = json!(false);
                    } else {
                        s.settings.select(Some("local-model"));
                    }
                    e.wait();
                    r.wait();
                    Ok::<(), String>(())
                })
            });
            entered.wait();
            let p = dir.path().to_owned();
            let second = std::thread::spawn(move || {
                update(&p, |s| {
                    if stop_first {
                        s.settings.select(Some("local-model"));
                    } else {
                        s.settings.api_mut()["enabled"] = json!(false);
                    }
                    Ok::<(), String>(())
                })
            });
            assert!(!second.is_finished());
            release.wait();
            first.join().unwrap().unwrap();
            second.join().unwrap().unwrap();
            assert_eq!(read(dir.path()).unwrap().api()["enabled"], false);
            assert_eq!(Settings::read(dir.path()).unwrap().api()["enabled"], false);
        }
        let dir = tempfile::tempdir().unwrap();
        let old = read(dir.path()).unwrap();
        assert!(update(dir.path(), |s| {
            s.cloud_epoch = 42;
            s.settings.select(Some("new"));
            Err::<(), String>("abort".into())
        })
        .is_err());
        assert_eq!(read(dir.path()).unwrap().0, old.0);
        assert_eq!(with_state(dir.path(), |s| s.cloud_epoch).unwrap(), 0);
        assert!(!dir.path().join("settings.json").exists());
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        assert!(update(dir.path(), |s| {
            s.cloud_epoch = 42;
            s.settings.select(Some("new"));
            Ok::<(), String>(())
        })
        .is_err());
        assert_eq!(read(dir.path()).unwrap().0, old.0);
        assert_eq!(with_state(dir.path(), |s| s.cloud_epoch).unwrap(), 0);
    }
    #[test]
    fn m1_migration_and_default_path_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"),json!({"version":1,"answer":{"local_pin_sha256":"existing","selection_initialised":true}}).to_string()).unwrap();
        assert_eq!(read(dir.path()).unwrap().default_path(), Some("local"));
        update(dir.path(), |s| {
            s.settings.select(None);
            Ok::<(), String>(())
        })
        .unwrap();
        assert_eq!(read(dir.path()).unwrap().default_path(), None);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).unwrap().default_path(), None);
        assert!(!dir.path().join("settings.json").exists());
        update(dir.path(), |s| {
            s.settings.select(Some("first"));
            Ok::<(), String>(())
        })
        .unwrap();
        assert_eq!(read(dir.path()).unwrap().default_path(), Some("local"));
    }
}
