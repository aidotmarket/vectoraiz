mod common;
use common::Fixture;
use std::{
    fs,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};
use vz_pack::export::{normalize_remote, sftp, SftpOptions};

struct Server {
    child: Child,
    port: u16,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn server(f: &Fixture, port: u16, host_key: &str) -> Option<Server> {
    let python = std::env::var("VZ_TEST_PYTHON")
        .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into());
    let spawn = Command::new(python)
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/sftp-test-server.py"
        ))
        .arg(&f.root)
        .args(["--port", &port.to_string(), "--host-key", host_key])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(error) => {
            eprintln!("SKIP SFTP server unavailable: {error}");
            assert!(
                std::env::var_os("CI").is_none(),
                "SFTP acceptance must run in Native CI"
            );
            return None;
        }
    };
    let output = child.stdout.take().unwrap();
    // Do not hang the test if Python cannot initialize the fixture.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output).read_line(&mut line);
        let _ = tx.send((result, line));
    });
    let ready = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .ok()
        .and_then(|(result, line)| {
            result
                .ok()
                .and_then(|_| serde_json::from_str::<serde_json::Value>(&line).ok())
        });
    if let Some(port) = ready.and_then(|v| v["port"].as_u64()) {
        Some(Server {
            child,
            port: port as u16,
        })
    } else {
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("SKIP SFTP test server could not start; install paramiko==3.5.1");
        assert!(
            std::env::var_os("CI").is_none(),
            "SFTP acceptance must run in Native CI"
        );
        None
    }
}

#[test]
fn paths_normalize_before_containment_check() {
    assert_eq!(normalize_remote("/source//./a/../b").unwrap(), "/source/b");
    assert!(normalize_remote("relative/path").is_err());
}

#[test]
fn local_sftp_acceptance() {
    let f = Fixture::new();
    let Some(server_a) = server(&f, 0, "host-a.pem") else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut options = SftpOptions {
        target: format!("fixture@127.0.0.1:{}:/source", server_a.port),
        staging: "/staging".into(),
        identity: Some(f.root.join("client.pem")),
        known_hosts: Some(f.root.join("pins.json")),
    };
    // A killed first-use pin write must not block the next attempt.
    fs::write(f.root.join("pins.lock"), b"").unwrap();
    fs::write(f.root.join("pins.pins.partial-stale"), b"incomplete").unwrap();
    let result = runtime
        .block_on(sftp(&f.pack, "fixture", &options))
        .unwrap();
    assert_eq!(result["mode"], "sftp");
    assert!(result["destination"]
        .as_str()
        .unwrap()
        .starts_with("sftp://fixture@127.0.0.1:"));
    f.assert_pack(&f.source.join("vz/fixture"), &result);
    let pins = fs::read(f.root.join("pins.json")).unwrap();
    let pinned: serde_json::Value = serde_json::from_slice(&pins).unwrap();
    assert!(pinned
        .as_object()
        .unwrap()
        .values()
        .all(|v| v.as_str().unwrap().starts_with("SHA256:")));
    // Same pin succeeds on a subsequent connection; existing pack is refused.
    assert!(format!(
        "{:#}",
        runtime
            .block_on(sftp(&f.pack, "fixture", &options))
            .unwrap_err()
    )
    .contains("already exists"));
    runtime.block_on(sftp(&f.pack, "second", &options)).unwrap();
    assert_eq!(fs::read(f.root.join("pins.json")).unwrap(), pins);
    options.identity = Some(f.root.join("encrypted.pem"));
    assert!(runtime
        .block_on(sftp(&f.pack, "encrypted", &options))
        .unwrap_err()
        .to_string()
        .contains("unencrypted"));
    options.identity = Some(f.root.join("client.pem"));
    options.staging = "/staging/../source//./nested".into();
    assert!(runtime
        .block_on(sftp(&f.pack, "inside", &options))
        .unwrap_err()
        .to_string()
        .contains("outside"));
    options.staging = "/staging".into();
    for (marker, message) in [
        ("bad-size", "remote size mismatch"),
        ("bad-hash", "remote hash mismatch"),
    ] {
        fs::write(f.root.join(marker), b"yes").unwrap();
        let error = runtime
            .block_on(sftp(&f.pack, marker, &options))
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
        assert!(!f.source.join("vz").join(marker).exists());
        fs::remove_file(f.root.join(marker)).unwrap();
    }
    fs::write(f.root.join("fail-rename"), b"yes").unwrap();
    let error = runtime
        .block_on(sftp(&f.pack, "retry", &options))
        .unwrap_err();
    assert!(error.to_string().contains("atomic SFTP rename failed"));
    assert!(!f.source.join("vz/retry").exists());
    assert!(fs::read_dir(&f.staging).unwrap().count() > 0);
    fs::remove_file(f.root.join("fail-rename")).unwrap();
    runtime.block_on(sftp(&f.pack, "retry", &options)).unwrap();
    let port = server_a.port;
    drop(server_a);
    // Same address and port, different server key: reject before authentication/upload.
    let server_b = server(&f, port, "host-b.pem").expect("restart fixture");
    let error = runtime
        .block_on(sftp(&f.pack, "mismatch", &options))
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("host key mismatch"),
        "{error:#}"
    );
    assert!(!f.source.join("vz/mismatch").exists());
    assert_eq!(fs::read(f.root.join("pins.json")).unwrap(), pins);
    drop(server_b);
}
