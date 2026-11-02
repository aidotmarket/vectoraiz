use crate::{claude_config, settings};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;
use vz_core::cloud_gate::{app_disclosure_version, AppClient, AppConsent, APP_DISCLOSURE};

#[derive(Default)]
pub struct Apps {
    pub app_access_lock: AsyncMutex<()>,
    pub channel: Arc<crate::mcp_channel::ChannelState>,
    log_lock: Mutex<()>,
    #[cfg(test)]
    pub claude_path: Option<std::path::PathBuf>,
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn disclosure(app: &str, name: &str) -> Value {
    let name = if app == "claude_desktop" {
        "Claude Desktop"
    } else {
        name
    };
    json!({"version":app_disclosure_version(),"strings":[APP_DISCLOSURE[0].replace("{App}",name),
        if app == "claude_desktop" { APP_DISCLOSURE[2].to_owned() } else { APP_DISCLOSURE[1].replace("{App}",name) },
        APP_DISCLOSURE[3].replace("{App}",name),APP_DISCLOSURE[4].replace("{App}",name),APP_DISCLOSURE[5].to_owned()]})
}
fn current(version: &str) -> Result<(), String> {
    if version != app_disclosure_version() {
        return Err("Open vectorAIz to confirm the updated terms.".into());
    }
    Ok(())
}
pub fn removed(name: &str) -> String {
    format!("{name} can no longer read your library. A result vectorAIz already sent may still arrive in {name}, and what it already has can't be taken back.")
}

/// Cancel and drain without ever holding the settings mutex across an await.
pub async fn deny(dir: &Path, ids: &[String], off: bool) -> Result<(), String> {
    let notify = settings::with_state(dir, |s| {
        s.app_off |= off;
        s.app_revoked.extend(ids.iter().cloned());
        for (client, token) in s.app_deliveries.values() {
            if off || ids.contains(client) {
                token.cancel();
            }
        }
        s.app_notify.clone()
    })?;
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if settings::with_state(dir, |s| {
            !s.app_deliveries
                .values()
                .any(|(id, _)| off || ids.contains(id))
        })? {
            break;
        }
        notified.await;
    }
    Ok(())
}
pub fn persist_denials(dir: &Path) -> Result<(), String> {
    settings::update(dir, |s| {
        let mut clients = s.app_state().clients;
        for c in &mut clients {
            if s.app_revoked.contains(&c.client_id) {
                c.revoked_at = Some(now());
            }
        }
        if s.app_off {
            s.settings.0["paths"]["app"]["enabled"] = json!(false);
        }
        s.settings.0["paths"]["app"]["clients"] = json!(clients);
        s.app_revoked.clear();
        s.app_off = false;
        Ok::<_, String>(())
    })
    .map(|_| ())
    .map_err(Into::into)
}
impl Apps {
    fn config_path(&self) -> Result<std::path::PathBuf, String> {
        #[cfg(test)]
        if let Some(path) = &self.claude_path {
            return Ok(path.clone());
        }
        claude_config::path()
    }

    pub fn status(&self, dir: &Path) -> Result<Value, String> {
        let mut value = settings::with_state(dir, |s| {
            let path = s.app_state();
            let clients: Vec<_> = path.clients.iter().map(|c|json!({"client_id":c.client_id,"app":c.app,"name":c.name,"created_at":c.created_at,
                "revoked":c.revoked_at.is_some() || s.app_revoked.contains(&c.client_id),"consent_current":c.consent.disclosure_version==app_disclosure_version()})).collect();
            let failed = s.app_off || !s.app_revoked.is_empty();
            let names = path
                .clients
                .iter()
                .filter(|c| {
                    s.app_revoked.contains(&c.client_id) || (s.app_off && c.revoked_at.is_none())
                })
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let names = if names.is_empty() {
                "Connected apps"
            } else {
                &names
            };
            json!({"enabled":path.enabled,"off":s.app_off,"clients":clients,"save_failed":if failed {Some(format!("vectorAIz couldn't save this change in {}. {} can't read your library until vectorAIz restarts. After a restart it can again unless the change is saved.",dir.display(), names))}else{None}})
        })?;
        value["channel_error"] = json!(self.channel.error.lock().unwrap().clone());
        value["recent"] = json!(self.recent(dir));
        Ok(value)
    }
    pub async fn connect(
        &self,
        dir: &Path,
        app: &str,
        name: &str,
        version: &str,
    ) -> Result<Value, String> {
        current(version)?;
        if name.trim().is_empty() || name.chars().count() > 100 {
            return Err("Enter an app name (up to 100 characters).".into());
        }
        let bridge = claude_config::bridge()?;
        let _access = self.app_access_lock.lock().await;
        let earlier = settings::with_state(dir, |s| {
            s.app_state()
                .clients
                .into_iter()
                .filter(|c| app == "claude_desktop" && c.app == app && c.revoked_at.is_none())
                .map(|c| c.client_id)
                .collect::<Vec<_>>()
        })?;
        deny(dir, &earlier, false).await?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| "Cannot create an AI app client id.")?;
        let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        settings::update(dir, |s| {
            let mut clients = s.app_state().clients;
            for c in &mut clients {
                if earlier.contains(&c.client_id) {
                    c.revoked_at = Some(now());
                }
            }
            let created_at = now();
            clients.push(AppClient {
                client_id: id.clone(),
                app: app.into(),
                name: name.trim().into(),
                consent: AppConsent {
                    disclosure_version: version.into(),
                    confirmed_at: created_at.clone(),
                },
                grants: vec!["library".into()],
                created_at,
                revoked_at: None,
            });
            s.settings.0["paths"]["app"] = json!({"enabled":true,"clients":clients});
            for old in &earlier {
                s.app_revoked.remove(old);
            }
            s.app_off = false;
            Ok::<_, String>(())
        })
        .map_err(String::from)?;
        let snippet = json!({"mcpServers":{"vectoraiz":claude_config::entry(&bridge,&id)}});
        // JSON string syntax also safely quotes TOML basic strings.
        Ok(
            json!({"client_id":id,"bridge":bridge,"json":serde_json::to_string_pretty(&snippet).unwrap(),
            "toml":format!("[mcp_servers.vectoraiz]\ncommand = {}\nargs = [\"--client\", {}]\n",json!(bridge),json!(id)),
            "command":format!("\"{}\" --client {}",bridge.display(),id)}),
        )
    }
    pub async fn confirm(&self, dir: &Path, id: &str, version: &str) -> Result<(), String> {
        let _access = self.app_access_lock.lock().await;
        current(version)?;
        settings::update(dir, |s| {
            let mut clients = s.app_state().clients;
            let c = clients
                .iter_mut()
                .find(|c| {
                    c.client_id == id && c.revoked_at.is_none() && !s.app_revoked.contains(id)
                })
                .ok_or(vz_mcp::NO_ACCESS)?;
            c.consent = AppConsent {
                disclosure_version: version.into(),
                confirmed_at: now(),
            };
            s.settings.0["paths"]["app"]["clients"] = json!(clients);
            Ok::<_, String>(())
        })
        .map(|_| ())
        .map_err(Into::into)
    }
    pub async fn disconnect(&self, dir: &Path, id: &str) -> Result<Value, String> {
        let _access = self.app_access_lock.lock().await;
        let client = settings::with_state(dir, |s| {
            s.app_state()
                .clients
                .into_iter()
                .find(|c| c.client_id == id)
        })?
        .ok_or(vz_mcp::NO_ACCESS)?;
        deny(dir, &[id.to_owned()], false).await?;
        // Persist ONLY this denial; never clear another client's failed revocation.
        let saved = settings::update(dir, |s| {
            let mut clients = s.app_state().clients;
            if let Some(c) = clients.iter_mut().find(|c| c.client_id == id) {
                c.revoked_at = Some(now());
            }
            s.settings.0["paths"]["app"]["clients"] = json!(clients);
            s.app_revoked.remove(id);
            Ok::<_, String>(())
        });
        let config = if client.app == "claude_desktop" {
            claude_config::edit(&self.config_path()?, id, None).err()
        } else {
            None
        };
        Ok(
            json!({"saved":saved.is_ok(),"message":removed(&client.name),"config_error":config,"status":self.status(dir)?}),
        )
    }
    pub async fn path_set(&self, dir: &Path, enabled: bool) -> Result<Value, String> {
        let _access = self.app_access_lock.lock().await;
        if !enabled {
            deny(dir, &[], true).await?;
            #[cfg(test)]
            crate::mcp_channel::pause(dir, "off_drained").await;
        }
        let result = settings::update(dir, |s| {
            s.settings.0["paths"]["app"]["enabled"] = json!(enabled);
            s.app_off = false;
            Ok::<_, String>(())
        });
        let saved = result.is_ok();
        if enabled {
            result.map_err(String::from)?;
        }
        Ok(
            json!({"saved":saved,"message":if !enabled {Some(removed("Connected apps"))}else{None},"status":self.status(dir)?}),
        )
    }
    pub async fn retry(&self, dir: &Path) -> Result<Value, String> {
        let _access = self.app_access_lock.lock().await;
        persist_denials(dir)?;
        self.status(dir)
    }
    pub fn record(&self, dir: &Path, client: &str, tool: &str, error: Option<&Value>) -> String {
        use std::io::Write;
        let call_id = uuid::Uuid::new_v4().simple().to_string();
        let tool = if ["list_collections", "search", "get_document", "get_chunk"].contains(&tool) {
            tool
        } else {
            "unknown_tool"
        };
        let prefix = if vz_mcp::valid_client(client) {
            client[..8].to_owned()
        } else {
            "invalid".into()
        };
        let _lock = self.log_lock.lock().unwrap();
        let name = settings::with_state(dir, |s| {
            if error.is_none() {
                *s.app_events.entry(client.to_owned()).or_default() += 1;
                s.app_notify.notify_waiters();
            }
            s.app_state()
                .clients
                .into_iter()
                .find(|c| c.client_id == client)
                .map(|c| c.name)
                .unwrap_or_else(|| "Unknown app".into())
        })
        .unwrap_or_else(|_| "Unknown app".into());
        if let Some(root) = dir.parent() {
            let _ = std::fs::create_dir_all(root);
            let path = root.join("mcp-calls.log");
            if std::fs::metadata(&path).is_ok_and(|m| m.len() >= 1024 * 1024) {
                let _ = std::fs::rename(&path, root.join("mcp-calls.log.old"));
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let v = json!({"call_id":call_id,"time":now(),"client":name,"client_id":prefix,"tool":tool,"allowed":error.is_none(),"refusal_code":error.map(|e|e["code"].clone()),"result_count":0});
                let _ = writeln!(f, "{v}");
            }
        }
        call_id
    }
    pub fn completed(&self, dir: &Path, call_id: &str, response: &Value) {
        let _lock = self.log_lock.lock().unwrap();
        let Some(root) = dir.parent() else {
            return;
        };
        for name in ["mcp-calls.log", "mcp-calls.log.old"] {
            let path = root.join(name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut lines: Vec<Value> = text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            let Some(entry) = lines.iter_mut().find(|v| v["call_id"] == call_id) else {
                continue;
            };
            let result = &response["result"];
            let count = result["hits"]
                .as_array()
                .or_else(|| result["collections"].as_array())
                .map_or(usize::from(response["ok"] == true), Vec::len);
            entry["result_count"] = json!(count);
            entry["refusal_code"] = response["code"].clone();
            let bytes = lines
                .into_iter()
                .map(|v| v.to_string() + "\n")
                .collect::<String>();
            let _ = std::fs::write(path, bytes);
            break;
        }
    }
    fn recent(&self, dir: &Path) -> Vec<Value> {
        let _lock = self.log_lock.lock().unwrap();
        dir.parent()
            .and_then(|p| std::fs::read_to_string(p.join("mcp-calls.log")).ok())
            .unwrap_or_default()
            .lines()
            .rev()
            .take(20)
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
    pub async fn test_connection(&self, dir: &Path, id: &str) -> Result<Value, String> {
        let (start, notify) = settings::with_state(dir, |s| {
            (
                s.app_events.get(id).copied().unwrap_or(0),
                s.app_notify.clone(),
            )
        })?;
        let wait = async {
            loop {
                let notified = notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if settings::with_state(dir, |s| {
                    s.app_events.get(id).copied().unwrap_or(0) > start
                })? {
                    return Ok::<_, String>(());
                }
                notified.await;
            }
        };
        if tokio::time::timeout(std::time::Duration::from_secs(60), wait)
            .await
            .is_ok_and(|r| r.is_ok())
        {
            return Ok(json!({"connected":true}));
        }
        self.connection_diagnosis(dir, id)
    }
    fn connection_diagnosis(&self, dir: &Path, id: &str) -> Result<Value, String> {
        let claude = settings::with_state(dir, |s| {
            s.app_state()
                .clients
                .iter()
                .any(|c| c.client_id == id && c.app == "claude_desktop")
        })?;
        if !claude {
            return Ok(json!({"connected":false,"reason":"no_call"}));
        }
        let entry = claude_config::read(&self.config_path()?)?["mcpServers"]["vectoraiz"].clone();
        let reason = if entry.is_null() {
            "entry_missing"
        } else if !claude_config::holds(&entry, id) {
            "another_client"
        } else if entry["command"] != json!(claude_config::bridge()?) {
            "stale_path"
        } else {
            "not_called"
        };
        Ok(json!({"connected":false,"reason":reason}))
    }
}
