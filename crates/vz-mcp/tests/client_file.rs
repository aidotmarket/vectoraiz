use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
struct Bridge {
    number: u64,
    child: Child,
    responses: mpsc::Receiver<Value>,
    stderr: std::fs::File,
}
impl Bridge {
    fn start(args: &[&str], home: &Path) -> Self {
        let stderr = tempfile::tempfile().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_vectoraiz-mcp"))
            .args(args)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr.try_clone().unwrap())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                if tx
                    .send(serde_json::from_str(&line.unwrap()).unwrap())
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            number: 0,
            child,
            responses,
            stderr,
        }
    }
    fn request(&mut self, method: &str) -> Value {
        self.number += 1;
        writeln!(self.child.stdin.as_mut().unwrap(),"{}",json!({"jsonrpc":"2.0","id":self.number,"method":method,"params":{"name":"search","arguments":{"query":"fixture"},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}})).unwrap();
        self.responses
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
    }
    fn stop(mut self) -> String {
        drop(self.child.stdin.take());
        assert!(self.child.wait().unwrap().success());
        use std::io::{Seek, SeekFrom};
        self.stderr.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        self.stderr.read_to_string(&mut text).unwrap();
        text
    }
}
#[test]
fn flags_and_invalid_files_preserve_discovery_and_deny_calls() {
    let root = tempfile::tempdir().unwrap();
    let both = Command::new(env!("CARGO_BIN_EXE_vectoraiz-mcp"))
        .args(["--client", "secret", "--client-file", "missing"])
        .output()
        .unwrap();
    assert!(!both.status.success());
    assert!(!String::from_utf8_lossy(&both.stderr).contains("secret"));
    let mut none = Bridge::start(&[], root.path());
    assert!(none.request("server/discover").get("result").is_some());
    assert_eq!(
        none.request("tools/call")["result"]["content"][0]["text"],
        vz_mcp::NO_ACCESS
    );
    none.stop();
    let path = root.path().join("client");
    let mut bridge = Bridge::start(&["--client-file", path.to_str().unwrap()], root.path());
    for contents in [
        None,
        Some("".to_owned()),
        Some("malformed".to_owned()),
        Some("a".repeat(257)),
        // Without the cap, trim() would accept the valid id after these spaces.
        Some(format!("{}{}", " ".repeat(300), "a".repeat(32))),
    ] {
        if let Some(text) = contents {
            std::fs::write(&path, text).unwrap();
        }
        assert!(vz_mcp::read_client_file(&path).is_empty());
        assert!(bridge.request("server/discover").get("result").is_some());
        assert!(bridge.request("tools/list").get("result").is_some());
        assert_eq!(
            bridge.request("tools/call")["result"]["content"][0]["text"],
            vz_mcp::NO_ACCESS
        );
    }
    let id = "a".repeat(32);
    for ending in ["\n", "\r\n"] {
        std::fs::write(&path, format!(" {id}{ending}")).unwrap();
        assert_eq!(vz_mcp::read_client_file(&path), id);
    }
    assert!(!bridge.stop().contains(&id));
}
#[cfg(unix)]
#[test]
fn running_bridge_rereads_replacement_and_relays_revocation_without_logging_ids() {
    use std::os::unix::net::UnixListener;
    let root = tempfile::Builder::new()
        .prefix("vz")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = root
        .path()
        .join("Library/Application Support/vectorAIz/mcp.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(socket).unwrap();
    let a = "a".repeat(32);
    let b = "b".repeat(32);
    let expected = [a.clone(), b.clone(), b.clone()];
    let server = std::thread::spawn(move || {
        for (i, id) in expected.iter().enumerate() {
            let (mut stream, _) = listener.accept().unwrap();
            let mut length = [0; 4];
            stream.read_exact(&mut length).unwrap();
            let mut bytes = vec![0; u32::from_le_bytes(length) as usize];
            stream.read_exact(&mut bytes).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap()["client"],
                *id
            );
            let value = if i == 2 {
                json!({"ok":false,"message":vz_mcp::NO_ACCESS})
            } else {
                json!({"ok":true,"result":{"hits":[]}})
            };
            let bytes = serde_json::to_vec(&value).unwrap();
            stream
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .unwrap();
            stream.write_all(&bytes).unwrap();
        }
    });
    let path = root.path().join("client");
    std::fs::write(&path, &a).unwrap();
    let mut bridge = Bridge::start(&["--client-file", path.to_str().unwrap()], root.path());
    assert_eq!(bridge.request("tools/call")["result"]["isError"], false);
    std::fs::write(&path, &b).unwrap();
    assert_eq!(bridge.request("tools/call")["result"]["isError"], false);
    assert_eq!(
        bridge.request("tools/call")["result"]["content"][0]["text"],
        vz_mcp::NO_ACCESS
    );
    let stderr = bridge.stop();
    assert!(!stderr.contains(&a) && !stderr.contains(&b));
    server.join().unwrap();
}
