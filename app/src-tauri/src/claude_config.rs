//! Preserve customer keys and file identity. Validate before backup or mutation.
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub fn path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    Ok(base
        .ok_or("Claude settings location unavailable")?
        .join("Claude/claude_desktop_config.json"))
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
pub fn read(path: &Path) -> Result<Value, String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({"mcpServers":{}})),
        Err(e) => return Err(e.to_string()),
    };
    let mut v: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Claude settings are not valid JSON")?;
    if !v.is_object() {
        return Err("Claude settings must be an object".into());
    }
    if v.get("mcpServers").is_none() {
        v["mcpServers"] = json!({});
    }
    if !v["mcpServers"].is_object() {
        return Err("Claude mcpServers must be an object".into());
    }
    Ok(v)
}
pub fn holds(entry: &Value, id: &str) -> bool {
    entry["args"]
        .as_array()
        .is_some_and(|args| args.windows(2).any(|w| w[0] == "--client" && w[1] == id))
}
pub fn edit(path: &Path, id: &str, bridge: Option<&Path>) -> Result<(), String> {
    let mut value = read(path)?;
    if let Some(bridge) = bridge {
        value["mcpServers"]["vectoraiz"] = entry(bridge, id);
    } else if holds(&value["mcpServers"]["vectoraiz"], id) {
        value["mcpServers"]
            .as_object_mut()
            .unwrap()
            .shift_remove("vectoraiz");
    } else {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    if path.exists() {
        fs::copy(
            path,
            path.with_file_name("claude_desktop_config.json.vectoraiz-backup"),
        )
        .map_err(|e| e.to_string())?;
    } else {
        fs::create_dir_all(path.parent().ok_or("Missing config directory")?)
            .map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
pub fn running() -> Result<bool, String> {
    #[cfg(windows)]
    let output = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq Claude.exe", "/FO", "CSV", "/NH"])
        .output();
    #[cfg(not(windows))]
    let output = std::process::Command::new("pgrep")
        .args(["-x", "Claude"])
        .output();
    let output = output.map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        if !output.status.success() {
            return Err("Cannot check whether Claude Desktop is open.".into());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|l| l.starts_with("\"Claude.exe\"")))
    }
    #[cfg(not(windows))]
    {
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err("Cannot check whether Claude Desktop is open.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shapes_backup_order_replacement_and_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("claude_desktop_config.json");
        edit(&path, "a", Some(Path::new("/bridge"))).unwrap();
        assert!(holds(&read(&path).unwrap()["mcpServers"]["vectoraiz"], "a"));
        let original = br#"{ "z": 1, "mcpServers": {"other":{"command":"elsewhere"},"vectoraiz":{"args":["--client","a"]}}, "a": 2 }"#;
        fs::write(&path, original).unwrap();
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&path).unwrap().ino()
        };
        edit(&path, "b", Some(Path::new("/new"))).unwrap();
        assert_eq!(
            fs::read(path.with_file_name("claude_desktop_config.json.vectoraiz-backup")).unwrap(),
            original
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        }
        assert_eq!(
            read(&path)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec!["z", "mcpServers", "a"]
        );
        let b = fs::read(&path).unwrap();
        edit(&path, "a", None).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b);
        edit(&path, "b", None).unwrap();
        assert!(read(&path).unwrap()["mcpServers"]["other"].is_object());
        for bad in ["{bad", "[]", r#"{"mcpServers":[]}"#] {
            fs::write(&path, bad).unwrap();
            assert!(edit(&path, "c", Some(Path::new("/b"))).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), bad);
        }
        fs::write(
            &path,
            r#"{"mcpServers":{"vectoraiz":{"args":["--client","b"]}}}"#,
        )
        .unwrap();
        edit(&path, "b", None).unwrap();
        assert_eq!(read(&path).unwrap()["mcpServers"], json!({}));
    }
}
