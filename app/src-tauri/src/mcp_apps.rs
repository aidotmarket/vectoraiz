use crate::local_time::format_local;
use crate::settings;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
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
    pub bridge_path: Option<PathBuf>,
    pub activity: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
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
        APP_DISCLOSURE[3].replace("{App}",name),APP_DISCLOSURE[4].replace("{App}",name),APP_DISCLOSURE[5].to_owned()],"extension":if app == "claude_desktop" {Some(APP_DISCLOSURE[6])}else{None}})
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
    fn changed(&self) {
        if let Some(emit) = self.activity.lock().unwrap().as_ref() {
            emit();
        }
    }
    pub fn status(&self, dir: &Path) -> Result<Value, String> {
        let last_calls = settings::with_state(dir, |s| s.app_state().clients)?
            .iter()
            .map(|c| (c.client_id.clone(), self.last_call(dir, c)))
            .collect::<std::collections::HashMap<_, _>>();
        let mut value = settings::with_state(dir, |s| {
            let path = s.app_state();
            let clients: Vec<_> = path.clients.iter().map(|c|json!({"client_id":c.client_id,"app":c.app,"name":c.name,"created_at":c.created_at,
                "revoked":c.revoked_at.is_some() || s.app_revoked.contains(&c.client_id),"consent_current":c.consent.disclosure_version==app_disclosure_version(),"last_call":last_calls.get(&c.client_id).cloned().flatten()})).collect();
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
        for client in value["clients"].as_array_mut().unwrap() {
            client["created_at_display"] = json!(format_local(
                client["created_at"].as_str().unwrap_or_default(),
                None
            ));
            client["last_call_display"] =
                json!(client["last_call"].as_str().map(|t| format_local(t, None)));
        }
        for entry in value["recent"].as_array_mut().unwrap() {
            entry["time_display"] = json!(format_local(
                entry["time"].as_str().unwrap_or_default(),
                None
            ));
        }
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
        let bridge = bridge()?;
        #[cfg(test)]
        let bridge = self.bridge_path.clone().unwrap_or(bridge);
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
        if app == "claude_desktop" {
            #[cfg(test)]
            crate::mcp_channel::pause(dir, "connect_before_file").await;
            return write_extension(dir, &bridge, &id);
        }
        let snippet = json!({"mcpServers":{"vectoraiz":entry(&bridge,&id)}});
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
        let file_error = if client.app == "claude_desktop" {
            let path = client_path(dir);
            if vz_mcp::read_client_file(&path) == id {
                std::fs::remove_file(&path)
                    .err()
                    .map(|_| format!("vectorAIz couldn't write {}.", path.display()))
            } else {
                None
            }
        } else {
            None
        };

        Ok(
            json!({"saved":saved.is_ok(),"message":if client.app == "claude_desktop" {format!("{} To remove vectorAIz from Claude Desktop too, open Settings → Desktop app → Extensions in Claude and uninstall it.",removed(&client.name))} else {removed(&client.name)},"file_error":file_error,"status":self.status(dir)?}),
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
        let _lock = self.log_lock.lock().unwrap();
        if let Some(root) = dir.parent() {
            let _ = std::fs::create_dir_all(root);
            let path = root.join("mcp-calls.log");
            if std::fs::metadata(&path).is_ok_and(|m| m.len() >= 1024 * 1024) {
                let _ = std::fs::rename(&path, root.join("mcp-calls.log.old"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                options.mode(0o600);
                for name in ["mcp-calls.log", "mcp-calls.log.old"] {
                    let _ = std::fs::set_permissions(
                        root.join(name),
                        std::fs::Permissions::from_mode(0o600),
                    );
                }
            }
            if let Ok(mut f) = options.open(path) {
                let v = json!({"call_id":call_id,"time":now(),"client":name,"client_id":prefix,"tool":tool,"allowed":error.is_none(),"refusal_code":error.map(|e|e["code"].clone()),"result_count":0});
                let _ = writeln!(f, "{v}");
            }
        }
        self.changed();
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
            self.changed();
            break;
        }
    }
    fn last_call(&self, dir: &Path, client: &AppClient) -> Option<String> {
        let _lock = self.log_lock.lock().unwrap();
        let created = chrono::DateTime::parse_from_rfc3339(&client.created_at).ok()?;
        ["mcp-calls.log", "mcp-calls.log.old"]
            .iter()
            .flat_map(|name| {
                std::fs::read_to_string(dir.parent().unwrap().join(name))
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .collect::<Vec<_>>()
            })
            .filter(|v| {
                v["allowed"] == true && v["client_id"].as_str() == client.client_id.get(..8)
            })
            .filter_map(|v| {
                let text = v["time"].as_str()?.to_owned();
                let time = chrono::DateTime::parse_from_rfc3339(&text).ok()?;
                (time >= created).then_some((time, text))
            })
            .max_by_key(|(time, _)| *time)
            .map(|(_, text)| text)
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
        if tokio::time::timeout(std::time::Duration::from_secs(120), wait)
            .await
            .is_ok_and(|r| r.is_ok())
        {
            return Ok(json!({"connected":true}));
        }
        Ok(json!({"connected":false,"reason":"no_call"}))
    }
}

pub fn bridge() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe
        .parent()
        .ok_or("App location unavailable")?
        .join(if cfg!(windows) {
            "vectoraiz-mcp.exe"
        } else {
            "vectoraiz-mcp"
        }))
}
pub fn entry(bridge: &Path, client: &str) -> Value {
    json!({"command":bridge,"args":["--client",client]})
}

pub fn client_path(dir: &Path) -> PathBuf {
    dir.parent()
        .unwrap()
        .join("ai-apps")
        .join("claude-desktop.client")
}
/// Opening is best effort: the response always retains the drag/install fallback.
pub fn open_extension(
    value: &Value,
    windows: bool,
    open: impl FnOnce(&str) -> Result<(), String>,
    reveal: impl FnOnce(&str) -> Result<(), String>,
) -> &'static str {
    let reveal_file = windows || value["moved"] == true;
    if let Some(path) = value["bundle"].as_str() {
        let _ = if reveal_file {
            reveal(path)
        } else {
            open(path)
        };
    }
    if reveal_file {
        "reveal"
    } else {
        "install"
    }
}
pub fn bundle_path(dir: &Path) -> PathBuf {
    dir.parent()
        .unwrap()
        .join("ai-apps")
        .join("vectoraiz-claude.mcpb")
}
pub fn manifest(bridge: &Path, client: &Path) -> Value {
    json!({"manifest_version":"0.3","name":"vectoraiz","display_name":"vectorAIz",
        "version":env!("CARGO_PKG_VERSION"),"description":"Search your vectorAIz library from Claude.",
        "author":{"name":"ai.market"},"server":{"type":"binary","entry_point":bridge.file_name().unwrap().to_str().unwrap(),
        "mcp_config":{"command":bridge,"args":["--client-file",client]}},
        "compatibility":{"platforms":[if cfg!(windows) {"win32"} else if cfg!(target_os="macos") {"darwin"} else {"linux"}]}})
}
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let folder = path.parent().unwrap();
    std::fs::create_dir_all(folder)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
    }
    // NamedTempFile uses create_new and mode 0600 on Unix. persist atomically replaces
    // an existing destination (including on Windows); neither file is truncated.
    let mut temp = tempfile::NamedTempFile::new_in(folder)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn write_extension(dir: &Path, bridge: &Path, id: &str) -> Result<Value, String> {
    use std::io::Write;
    let client = client_path(dir);
    let bundle = bundle_path(dir);
    let failed = |p: &Path| format!("vectorAIz couldn't write {}.", p.display());
    atomic_write(&client, id.as_bytes()).map_err(|_| failed(&client))?;
    let previous = std::fs::read(&bundle).ok().and_then(|bytes| {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
        let mut file = archive.by_name("manifest.json").ok()?;
        serde_json::from_reader::<_, Value>(&mut file).ok()
    });
    let moved = previous.is_some_and(|v| v["server"]["mcp_config"]["command"] != json!(bridge));
    let result = (|| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest(bridge, &client))?)?;
        zip.start_file(bridge.file_name().unwrap().to_str().unwrap(), options)?;
        zip.write_all(&std::fs::read(bridge)?)?;
        Ok(zip.finish()?.into_inner())
    })();
    atomic_write(&bundle, &result.map_err(|_| failed(&bundle))?).map_err(|_| failed(&bundle))?;
    Ok(json!({"client_id":id,"bundle":bundle,"moved":moved}))
}

#[cfg(test)]
mod extension_tests {
    use super::*;
    #[test]
    fn atomic_private_client_and_manifest_bundle() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("models");
        let bridge = root.path().join(if cfg!(windows) {
            "vectoraiz-mcp.exe"
        } else {
            "vectoraiz-mcp"
        });
        std::fs::write(&bridge, b"bridge fixture").unwrap();
        let a = "a".repeat(32);
        let b = "b".repeat(32);
        let client = root.path().join("ai-apps").join("claude-desktop.client");
        let bundle = root.path().join("ai-apps").join("vectoraiz-claude.mcpb");
        assert_eq!(client_path(&dir), client);
        assert_eq!(bundle_path(&dir), bundle);
        let first = write_extension(&dir, &bridge, &a).unwrap();
        assert_eq!(first["bundle"], json!(bundle));
        assert_eq!(first["moved"], false);
        let bytes = std::fs::read(bundle_path(&dir)).unwrap();
        write_extension(&dir, &bridge, &b).unwrap();
        assert_eq!(vz_mcp::read_client_file(&client_path(&dir)), b);
        assert_eq!(std::fs::read(bundle_path(&dir)).unwrap(), bytes);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        for i in 0..zip.len() {
            assert!(!zip.by_index(i).unwrap().name().contains('\\'));
        }
        let mut file = zip.by_name("manifest.json").unwrap();
        let value: Value = serde_json::from_reader(&mut file).unwrap();
        assert_eq!(value, manifest(&bridge, &client_path(&dir)));
        assert_eq!(value["server"]["mcp_config"]["command"], json!(bridge));
        assert_eq!(
            value["server"]["mcp_config"]["args"],
            json!(["--client-file", client])
        );
        drop(file);
        assert!(zip
            .by_name(bridge.file_name().unwrap().to_str().unwrap())
            .is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let path = client_path(&dir);
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            let old = std::fs::File::open(&path).unwrap();
            atomic_write(&path, a.as_bytes()).unwrap();
            assert_ne!(
                old.metadata().unwrap().ino(),
                std::fs::metadata(&path).unwrap().ino()
            );
        }
        let moved = root.path().join("moved-bridge");
        std::fs::write(&moved, b"bridge fixture").unwrap();
        assert_eq!(write_extension(&dir, &moved, &b).unwrap()["moved"], true);
    }
    #[test]
    fn open_or_reveal_choice_and_failure_keep_fallback() {
        for windows in [false, true] {
            for moved in [false, true] {
                let value = json!({"bundle":"bundle.mcpb", "moved":moved});
                let expected_reveal = windows || moved;
                let calls = std::cell::Cell::new(0);
                let opened = open_extension(
                    &value,
                    windows,
                    |path| {
                        assert!(!expected_reveal);
                        calls.set(calls.get() + 1);
                        assert_eq!(path, "bundle.mcpb");
                        Err("no handler".into())
                    },
                    |path| {
                        assert!(expected_reveal);
                        calls.set(calls.get() + 1);
                        assert_eq!(path, "bundle.mcpb");
                        Err("no handler".into())
                    },
                );
                assert_eq!(calls.get(), 1);
                assert_eq!(opened, if expected_reveal { "reveal" } else { "install" });
                assert_eq!(value["bundle"], "bundle.mcpb");
            }
        }
    }
    // icacls prints the requested filename, then one principal:(flags)(rights)
    // per line, followed by a blank line and its processing summary.
    fn private_acl_entries(acl: &str, name: &str, user: &str) -> Result<Vec<Value>, String> {
        let rules = acl.strip_prefix(name).ok_or("missing icacls path header")?;
        let mut entries = Vec::new();
        let mut allows = 0;
        for line in rules.lines().take_while(|line| !line.trim().is_empty()) {
            let (principal, flags) = line.trim().split_once(":(").ok_or("invalid ACL entry")?;
            let permissions = format!("({flags}");
            let allow = !permissions.contains("(DENY)");
            if allow {
                allows += 1;
                if !["NT AUTHORITY\\SYSTEM", "BUILTIN\\Administrators", user]
                    .iter()
                    .any(|allowed| principal.eq_ignore_ascii_case(allowed))
                {
                    return Err(format!("unexpected access-allow principal: {principal}"));
                }
            }
            entries.push(json!({"principal": principal, "allow": allow,
                "inherited": permissions.contains("(I)"), "permissions": permissions}));
        }
        if allows == 0 {
            return Err("no access-allow entries observed".into());
        }
        Ok(entries)
    }
    #[test]
    fn private_acl_accepts_explicit_and_inherited_entries() {
        let acl = "client NT AUTHORITY\\SYSTEM:(F)\n  BUILTIN\\Administrators:(I)(F)\n  HOST\\alice:(F)\n\nSuccessfully processed 1 files";
        for acl in [acl.to_owned(), acl.replace('\n', "\r\n")] {
            let entries = private_acl_entries(&acl, "client", "host\\alice").unwrap();
            assert_eq!(entries.len(), 3);
            assert_eq!(entries[0]["inherited"], false);
            assert_eq!(entries[1]["inherited"], true);
        }
        assert!(private_acl_entries("client \n\n", "client", "host\\alice").is_err());
    }
    #[test]
    fn private_acl_rejects_every_other_allow_principal() {
        for principal in [
            "Everyone",
            "BUILTIN\\Users",
            "NT AUTHORITY\\Authenticated Users",
            "HOST\\another-user",
        ] {
            for flags in ["(F)", "(I)(RX)", "(OI)(CI)(IO)(F)"] {
                let acl = format!("client HOST\\alice:(F)\n  {principal}:{flags}\n\n");
                assert!(private_acl_entries(&acl, "client", "host\\alice").is_err());
            }
        }
        let acl = "client HOST\\alice:(F)\n  Everyone:(I)(DENY)(W)\n\n";
        assert!(private_acl_entries(acl, "client", "host\\alice").is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn client_file_and_folder_have_user_only_acl() {
        let output = std::process::Command::new("whoami").output().unwrap();
        assert!(output.status.success(), "whoami failed");
        let user = String::from_utf8(output.stdout).unwrap();
        let user = user.trim();
        assert!(!user.is_empty(), "whoami returned no current user");
        let profile = std::env::var_os("USERPROFILE").unwrap();
        let root = tempfile::tempdir_in(profile).unwrap();
        let client = root.path().join("ai-apps").join("claude-desktop.client");
        atomic_write(&client, b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let mut observations = Vec::new();
        let mut failures = Vec::new();
        let mut observe = |stage: &str, path: &Path| {
            // A relative filename makes the header independent of path separators.
            let name = path.file_name().unwrap().to_str().unwrap();
            let output = std::process::Command::new("icacls")
                .current_dir(path.parent().unwrap())
                .arg(name)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "ACL observation failed for {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            let acl = String::from_utf8(output.stdout).expect("icacls output must be UTF-8");
            let entries = private_acl_entries(&acl, name, user);
            if let Err(error) = &entries {
                failures.push(format!("{}: {error}: {acl}", path.display()));
            }
            observations.push(json!({"stage": stage, "path": path, "current_user": user,
                "icacls": acl, "entries": entries.as_ref().ok(), "error": entries.as_ref().err()}));
        };
        observe("atomic_write", &client);
        observe("folder", client.parent().unwrap());
        // Diagnose creation versus rename without modifying the production writer.
        // tempfile 3.27 uses OpenOptions (no custom security descriptor), then
        // SetFileAttributesW + MoveFileExW(REPLACE_EXISTING) on the same volume.
        let temp = tempfile::NamedTempFile::new_in(client.parent().unwrap()).unwrap();
        observe("before_persist", temp.path());
        let persisted = client.with_file_name("acl-probe");
        temp.persist(&persisted).unwrap();
        observe("after_persist", &persisted);
        atomic_write(&client, b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        observe("atomic_replace", &client);
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("mcpb-acl.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, serde_json::to_vec_pretty(&observations).unwrap()).unwrap();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
    #[test]
    fn generated_manifest_for_pinned_ci_validator() {
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("mcpb-manifest.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let value = manifest(
            &bridge().unwrap(),
            &target.parent().unwrap().join("claude-desktop.client"),
        );
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        assert!(!bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        std::fs::write(target, bytes).unwrap();
    }
}
