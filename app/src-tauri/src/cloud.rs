//! Serialized API-key lifecycle. Settings closures contain no I/O or awaits.
use crate::settings::{self, Mark, Operation, SettingsState};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{watch, Mutex};
use vz_core::{
    cloud_gate::{self, Op, Provider, SendError},
    credentials::{CredError, NativeStore, Secret, Store},
};

pub struct Cloud {
    pub cloud_lock: Mutex<()>,
    pub store: Arc<dyn Store>,
    #[cfg(test)]
    pub hook: Arc<dyn Fn(&str) + Send + Sync>,
}
impl Default for Cloud {
    fn default() -> Self {
        Self {
            cloud_lock: Mutex::new(()),
            store: Arc::new(NativeStore),
            #[cfg(test)]
            hook: Arc::new(|_| {}),
        }
    }
}
pub struct Running {
    pub dir: PathBuf,
    pub id: String,
    pub epoch: u64,
    pub cancel: watch::Receiver<bool>,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = settings::with_state(&self.dir, |s| s.ops.remove(&self.id));
    }
}
pub fn register(dir: &Path, kind: &'static str) -> Result<Running, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = watch::channel(false);
    let epoch = settings::with_state(dir, |s| {
        if kind == "cloud" && s.ops.values().any(|o| o.kind == kind) {
            return Err("An answer or key check is already running".to_owned());
        }
        let epoch = s.cloud_epoch;
        s.ops.insert(
            id.clone(),
            Operation {
                kind,
                epoch,
                mark: Mark::Running,
                cancel: tx,
            },
        );
        Ok(epoch)
    })??;
    Ok(Running {
        dir: dir.to_owned(),
        id,
        epoch,
        cancel: rx,
    })
}
fn mark(state: &SettingsState, id: &str) -> Result<(), String> {
    match state.ops.get(id).map(|o| o.mark) {
        Some(Mark::Running) => Ok(()),
        Some(Mark::Committed) => Err("already finished".into()),
        Some(Mark::Cancelled) => Err("cancelled".into()),
        _ => Err("revoked".into()),
    }
}
pub fn cancel(dir: &Path, kind: &str) -> Result<(), String> {
    let (senders, finished) = settings::with_state(dir, |s| {
        let mut senders = vec![];
        let mut finished = false;
        for o in s.ops.values_mut().filter(|o| o.kind == kind) {
            if o.mark == Mark::Running {
                o.mark = Mark::Cancelled;
                senders.push(o.cancel.clone());
            } else if o.mark == Mark::Committed {
                finished = true;
            }
        }
        (senders, finished)
    })?;
    for tx in senders {
        let _ = tx.send(true);
    }
    if finished {
        Err("already finished".into())
    } else {
        Ok(())
    }
}
pub fn revoke(dir: &Path, stop: bool) -> Result<(), String> {
    let senders = settings::with_state(dir, |s| {
        s.cloud_epoch += 1;
        if stop {
            s.cloud_stopped = true;
        }
        s.ops
            .values_mut()
            .filter_map(|o| {
                if o.mark == Mark::Running {
                    o.mark = Mark::Revoked;
                    Some(o.cancel.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    })?;
    for tx in senders {
        let _ = tx.send(true);
    }
    Ok(())
}
pub fn snapshot(dir: &Path) -> (cloud_gate::ConsentState, u64) {
    settings::with_state(dir, |s| (s.consent_state(), s.cloud_epoch)).unwrap_or_default()
}
pub fn status(dir: &Path) -> Result<Value, String> {
    settings::with_state(dir, |s| {
        let state = s.consent_state();
        let current = state.provider.is_some_and(|p| {
            state.consent.as_ref().is_some_and(|c| {
                c.provider == p && c.disclosure_version == cloud_gate::disclosure_version()
            })
        });
        json!({"default":s.settings.0["paths"]["default"],"api_key":{"enabled":state.enabled&&!state.stopped,"provider":state.provider,"key_present":state.key_present,"key_last4":s.settings.api()["key_last4"]},"consent_current":current,"consent_provider":state.consent.map(|c|c.provider),"cleanup_pending":pending(s),"stop_failed":s.stop_failed,"stopped":state.stopped,"message":targets(s).first().map(|p|cleanup_message(*p))})
    })
}
fn pending(s: &SettingsState) -> bool {
    !s.settings.api()["pending"].is_null()
        || s.settings.api()["cleanup"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
}
fn targets(s: &SettingsState) -> Vec<Provider> {
    let mut result = vec![];
    if let Ok(p) = serde_json::from_value(s.settings.api()["pending"].clone()) {
        result.push(p);
    }
    if let Some(a) = s.settings.api()["cleanup"].as_array() {
        for v in a {
            if let Ok(p) = serde_json::from_value(v.clone()) {
                if !result.contains(&p) {
                    result.push(p);
                }
            }
        }
    }
    result
}
fn cleanup_add(s: &mut SettingsState, p: Provider) {
    let a = s.settings.api_mut()["cleanup"]
        .as_array_mut()
        .expect("cleanup list");
    let p = json!(p);
    if !a.contains(&p) {
        a.push(p);
    }
}
impl Cloud {
    async fn remove(&self, p: Provider) -> Result<(), CredError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.remove(p))
            .await
            .map_err(|_| CredError::Unavailable)?
    }
    pub async fn reconcile(&self, dir: &Path) -> Result<(), String> {
        let accounts = settings::with_state(dir, |s| targets(s))?;
        for p in accounts {
            if self.remove(p).await.is_ok() {
                settings::update(dir, |s| {
                    if s.settings.api()["pending"] == json!(p) {
                        s.settings.api_mut()["pending"] = Value::Null;
                    }
                    if let Some(a) = s.settings.api_mut()["cleanup"].as_array_mut() {
                        a.retain(|v| *v != json!(p));
                    }
                    if s.settings.api()["provider"] == json!(p) {
                        s.settings.api_mut()["key_present"] = json!(false);
                        s.settings.api_mut()["key_last4"] = Value::Null;
                    }
                    Ok::<(), String>(())
                })?;
            }
        }
        Ok(())
    }
    pub async fn confirm(&self, dir: &Path, p: Provider, version: String) -> Result<Value, String> {
        if version != cloud_gate::disclosure_version() {
            return Ok(
                json!({"kind":"cloud_error","reason":"consent_required","disclosure":cloud_gate::disclosure(p)}),
            );
        }
        if settings::with_state(dir, |s| s.settings.api()["provider"] != json!(p))? {
            revoke(dir, false)?;
        }
        let _lock = self.cloud_lock.lock().await;
        settings::update(dir, |s| {
            s.settings.api_mut()["consent"] =
                json!({"provider":p,"disclosure_version":version,"confirmed_at":now()});
            Ok::<(), String>(())
        })?;
        status(dir)
    }
    pub async fn set(&self, dir: &Path, p: Provider, key: Secret) -> Result<Value, String> {
        #[cfg(test)]
        (self.hook)("before_register");
        let mut op = register(dir, "cloud_setup")?;
        let _lock = self.cloud_lock.lock().await;
        self.reconcile(dir).await?;
        if let Some(account) = settings::with_state(dir, |s| targets(s).first().copied())? {
            return Err(cleanup_message(account));
        }
        op.epoch = settings::with_state(dir, |s| {
            mark(s, &op.id)?;
            if s.stop_failed {
                return Err("Restart vectorAIz to set up a key again.".to_owned());
            }
            cloud_gate::check(&s.consent_state(), p, Op::KeyCheck)
                .map_err(|e| SendError::from(e).reason().to_owned())?;
            let epoch = s.cloud_epoch;
            s.ops.get_mut(&op.id).unwrap().epoch = epoch;
            Ok(epoch)
        })??;
        #[cfg(test)]
        (self.hook)("key_check");
        #[cfg(not(test))]
        let checked = cloud_gate::send(
            || snapshot(dir),
            p,
            Op::KeyCheck,
            &key,
            None,
            &op.epoch,
            &mut op.cancel,
        )
        .await;
        #[cfg(test)]
        let checked: Result<(), SendError> = if key.last4() == "ject" {
            Err(SendError::KeyRejected)
        } else {
            Ok(())
        };
        if let Err(e) = checked {
            settings::with_state(dir, |s| mark(s, &op.id))??;
            return Err(if e == SendError::KeyRejected {
                "That key was not accepted.".into()
            } else {
                e.reason().into()
            });
        }
        settings::update(dir, |s| {
            mark(s, &op.id)?;
            cloud_gate::check(&s.consent_state(), p, Op::KeyCheck)
                .map_err(|_| "revoked".to_owned())?;
            s.settings.api_mut()["pending"] = json!(p);
            Ok::<(), String>(())
        })?;
        #[cfg(test)]
        (self.hook)("intent");
        let last4 = key.last4();
        let store = self.store.clone();
        // Never select against a started blocking credential call. Await, compensate,
        // then release cloud_lock even when cancellation arrived during the OS prompt.
        let stored = tokio::task::spawn_blocking(move || store.store(p, &key))
            .await
            .map_err(|_| CredError::Unavailable)
            .and_then(|r| r);
        if stored.is_err() {
            let _ = self.reconcile(dir).await;
            return Err(credential_message(p));
        }
        #[cfg(test)]
        (self.hook)("before_commit");
        let commit = settings::update(dir, |s| {
            mark(s, &op.id)?;
            cloud_gate::check(&s.consent_state(), p, Op::KeyCheck)
                .map_err(|_| "revoked".to_owned())?;
            #[cfg(test)]
            (self.hook)("commit");
            let old = serde_json::from_value::<Provider>(s.settings.api()["provider"].clone()).ok();
            let api = s.settings.api_mut();
            api["pending"] = Value::Null;
            api["provider"] = json!(p);
            api["key_present"] = json!(true);
            api["key_last4"] = json!(last4);
            api["enabled"] = json!(true);
            if s.settings.default_path().is_none() {
                s.settings.0["paths"]["default"] = json!("api_key");
            }
            if let Some(old) = old.filter(|old| *old != p) {
                cleanup_add(s, old);
            }
            s.cloud_epoch += 1;
            s.ops.get_mut(&op.id).unwrap().mark = Mark::Committed;
            Ok::<(), String>(())
        });
        if let Err(error) = commit {
            let _ = self.reconcile(dir).await;
            return Err(error.to_string());
        }
        #[cfg(test)]
        (self.hook)("after_commit");
        let senders = settings::with_state(dir, |s| {
            s.ops
                .values()
                .filter(|o| o.kind == "cloud" && o.epoch < s.cloud_epoch)
                .map(|o| o.cancel.clone())
                .collect::<Vec<_>>()
        })?;
        for tx in senders {
            let _ = tx.send(true);
        }
        self.reconcile(dir).await?;
        status(dir)
    }
    pub async fn stop(&self, dir: &Path) -> Result<Value, String> {
        revoke(dir, true)?;
        let _lock = self.cloud_lock.lock().await;
        let (mut accounts, provider) = settings::with_state(dir, |s| {
            let mut a = targets(s);
            let p = serde_json::from_value::<Provider>(s.settings.api()["provider"].clone()).ok();
            if let Some(p) = p {
                if !a.contains(&p) {
                    a.push(p);
                }
            }
            (a, p)
        })?;
        let stopped = settings::update(dir, |s| {
            for p in &accounts {
                cleanup_add(s, *p);
            }
            let api = s.settings.api_mut();
            api["pending"] = Value::Null;
            api["enabled"] = json!(false);
            api["key_present"] = json!(false);
            api["key_last4"] = Value::Null;
            s.settings.0["paths"]["default"] = if s.settings.selected().is_some() {
                json!("local")
            } else {
                Value::Null
            };
            s.cloud_stopped = false;
            Ok::<(), String>(())
        });
        if stopped.is_err() {
            settings::with_state(dir, |s| s.stop_failed = true)?;
            let mut removed = true;
            for p in accounts.drain(..) {
                removed &= self.remove(p).await.is_ok();
            }
            let mut result = status(dir)?;
            result["message"] = json!(format!(
                "vectorAIz couldn't save this change in {}. Sending is off until you restart{}",
                dir.display(),
                if removed {
                    ", and your key was removed."
                } else {
                    "."
                }
            ));
            return Ok(result);
        }
        self.reconcile(dir).await?;
        let mut result = status(dir)?;
        if let Some(p) = provider {
            result["message"] = json!(if result["cleanup_pending"] == true {
                cleanup_message(p)
            } else {
                cloud_gate::stopped_message(p)
            });
        }
        Ok(result)
    }
    pub async fn retry(&self, dir: &Path) -> Result<Value, String> {
        let _lock = self.cloud_lock.lock().await;
        self.reconcile(dir).await?;
        status(dir)
    }
    /// Called only by pipeline_ask, while holding cloud_lock, after reconciliation.
    pub async fn load_for_ask(&self, op: &Running, p: Provider) -> Result<Secret, String> {
        let _lock = self.cloud_lock.lock().await;
        self.reconcile(&op.dir).await?;
        let s = settings::with_state(&op.dir, |s| s.clone())?;
        mark(&s, &op.id)?;
        if s.cloud_epoch != op.epoch {
            return Err("revoked".into());
        }
        if !s.consent_state().key_present {
            return Err("no_key".into());
        }
        if pending(&s) {
            return Err(cleanup_message(p));
        }
        cloud_gate::check(&s.consent_state(), p, Op::Ask)
            .map_err(|e| SendError::from(e).reason().to_owned())?;
        let store = self.store.clone();
        let key = tokio::task::spawn_blocking(move || store.load(p))
            .await
            .map_err(|_| credential_message(p))?;
        settings::with_state(&op.dir, |s| mark(s, &op.id))??;
        match key {
            Err(CredError::Missing) => {
                settings::update(&op.dir, |s| {
                    s.settings.api_mut()["key_present"] = json!(false);
                    s.settings.api_mut()["key_last4"] = Value::Null;
                    Ok::<(), String>(())
                })?;
                Err("no_key".into())
            }
            Err(_) => Err(credential_message(p)),
            Ok(key) => Ok(key),
        }
    }
}
pub fn path_set(dir: &Path, path: &str) -> Result<Value, String> {
    settings::update(dir, |s| {
        match path {
            "local" if s.settings.selected().is_some() => {}
            "api_key" => {
                let p = s.consent_state().provider.ok_or("no_key")?;
                cloud_gate::check(&s.consent_state(), p, Op::Ask)
                    .map_err(|e| SendError::from(e).reason().to_owned())?;
            }
            _ => return Err("Set up this answer path first.".into()),
        }
        s.settings.0["paths"]["default"] = json!(path);
        Ok::<(), String>(())
    })?;
    status(dir)
}
pub fn credential_message(p: Provider) -> String {
    format!("vectorAIz couldn't read your {} key from this computer's password store. Allow access when asked, or add the key again in Settings.",p.name())
}
pub fn cleanup_message(p: Provider) -> String {
    format!("Your {} key couldn't be removed from this computer's password store yet. vectorAIz will try again.",p.name())
}
fn now() -> String {
    // UTC RFC3339, without another time dependency.
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86400) as i64;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    if m <= 2 {
        y += 1;
    }
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        seconds % 86400 / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}
#[cfg(test)]
mod tests;
