//! Exercise the shipped bridge process over real local streams and stdio.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
use tokio::io::AsyncReadExt;

struct Client {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Value>,
    modern: bool,
}
impl Client {
    fn raw(home: &std::path::Path, modern: bool) -> (Self, std::process::ChildStdout) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vectoraiz-mcp"))
            .args(["--client", &"a".repeat(32)])
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (_, output) = mpsc::channel();
        (
            Self {
                child,
                input,
                output,
                modern,
            },
            stdout,
        )
    }
    fn new(home: &std::path::Path, modern: bool) -> Self {
        let (mut client, stdout) = Self::raw(home, modern);
        let (send, output) = mpsc::channel();
        client.output = output;
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                send.send(serde_json::from_str(&line.unwrap()).unwrap())
                    .unwrap();
            }
        });
        client
    }
    fn write(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }
    fn request(&mut self, id: u64, method: &str, mut params: Value) {
        if self.modern {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}});
        }
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
    }
    fn response(&self, id: u64) -> Value {
        let response = self.output.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(response["id"], id);
        if self.modern && response.get("result").is_some() {
            assert_eq!(response["result"]["resultType"], "complete");
        }
        response
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn same_bridge_revocation_cancellation_and_handoff_both_eras() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    #[cfg(unix)]
    let home = tempfile::Builder::new()
        .prefix("vz-mcp-")
        .tempdir_in("/tmp")
        .unwrap();
    #[cfg(windows)]
    let home = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let listener = runtime
        .block_on(vz_mcp::channel::bind_at(
            &home
                .path()
                .join("Library/Application Support/vectorAIz/mcp.sock"),
        ))
        .unwrap();
    #[cfg(windows)]
    let mut listener = runtime.block_on(async { vz_mcp::channel::create_server(true).unwrap() });
    for modern in [true, false] {
        // Real stdout backpressure, beyond Tokio's 2 MiB buffer. Read only a
        // prefix until the bridge has processed cancellation and another call.
        {
            let (mut blocked, mut stdout) = Client::raw(home.path(), modern);
            blocked.request(1, if modern { "server/discover" } else { "initialize" }, json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], 1);
            stdout = reader.into_inner();
            if !modern {
                blocked.write(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
            }
            blocked.request(
                2,
                "tools/call",
                json!({"name":"get_document","arguments":{"doc_id":"d"}}),
            );
            runtime.block_on(async {
                #[cfg(unix)]
                let mut stream = listener.accept().await.unwrap().0;
                #[cfg(windows)]
                let mut stream = {
                    listener.connect().await.unwrap();
                    let next = vz_mcp::channel::create_server(false).unwrap();
                    std::mem::replace(&mut listener, next)
                };
                vz_mcp::channel::read_frame(&mut stream, vz_mcp::REQUEST_CAP)
                    .await
                    .unwrap();
                let response = json!({"ok":true,"result":{"doc_id":"d","file_name":"document.txt","page_count":1,"truncated":false,"text":"\u{0001}".repeat(200_000)}});
                vz_mcp::channel::write_frame(
                    &mut stream,
                    &serde_json::to_vec(&response).unwrap(),
                    vz_mcp::RESPONSE_CAP,
                )
                .await
                .unwrap();
            });
            let mut prefix = vec![0; 1024];
            stdout.read_exact(&mut prefix).unwrap();
            blocked.write(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}));
            blocked.request(3, "tools/call", json!({"name":"list_collections"}));
            runtime.block_on(async {
                #[cfg(unix)]
                let mut stream = listener.accept().await.unwrap().0;
                #[cfg(windows)]
                let mut stream = {
                    listener.connect().await.unwrap();
                    let next = vz_mcp::channel::create_server(false).unwrap();
                    std::mem::replace(&mut listener, next)
                };
                let request = vz_mcp::channel::read_frame(&mut stream, vz_mcp::REQUEST_CAP)
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::from_slice::<Value>(&request).unwrap()["args"],
                    json!({})
                );
                vz_mcp::channel::write_frame(
                    &mut stream,
                    br#"{"ok":true,"result":{"collections":[]}}"#,
                    vz_mcp::RESPONSE_CAP,
                )
                .await
                .unwrap();
            });
            let mut reader = BufReader::new(stdout);
            reader.read_until(b'\n', &mut prefix).unwrap();
            assert!(prefix.len() > 2 * 1024 * 1024);
            assert_eq!(serde_json::from_slice::<Value>(&prefix).unwrap()["id"], 2);
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], 3);
        }
        let mut client = Client::new(home.path(), modern);
        if modern {
            client.request(1, "server/discover", json!({}));
        } else {
            client.request(1,"initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
        }
        client.response(1);
        if !modern {
            client.write(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        }
        for invalid in [
            Value::Null,
            json!([]),
            json!("invalid"),
            json!(1),
            json!(true),
        ] {
            client.request(
                20,
                "tools/call",
                json!({"name":"list_collections","arguments":invalid}),
            );
            assert_eq!(client.response(20)["error"]["code"], -32602);
        }
        client.request(21, "tools/call", json!({"name":"list_collections"}));
        runtime.block_on(async {
            #[cfg(unix)]
            let mut stream = listener.accept().await.unwrap().0;
            #[cfg(windows)]
            let mut stream = {
                listener.connect().await.unwrap();
                let next = vz_mcp::channel::create_server(false).unwrap();
                std::mem::replace(&mut listener, next)
            };
            let bytes = vz_mcp::channel::read_frame(&mut stream, vz_mcp::REQUEST_CAP).await.unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["args"], json!({}));
            assert_eq!(request["client"], "a".repeat(32));
            let response = json!({"ok":true,"result":{"collections":[{"id":"library","name":"Documents","documents":1}]}});
            vz_mcp::channel::write_frame(&mut stream, &serde_json::to_vec(&response).unwrap(), vz_mcp::RESPONSE_CAP).await.unwrap();
        });
        assert_eq!(
            client.response(21)["result"]["structuredContent"]["collections"][0]["id"],
            "library"
        );
        for (id, phase) in [
            (2, "granted"),
            (3, "removed"),
            (4, "lock_wait"),
            (6, "after_tool"),
            (8, "handoff"),
        ] {
            client.request(
                id,
                "tools/call",
                json!({"name":"search","arguments":{"query":"private query"}}),
            );
            runtime.block_on(async {
                #[cfg(unix)]
                let mut stream=tokio::time::timeout(Duration::from_secs(5),listener.accept()).await.unwrap().unwrap().0;
                #[cfg(windows)]
                let mut stream={
                    tokio::time::timeout(Duration::from_secs(5),listener.connect()).await.unwrap().unwrap();
                    let next=vz_mcp::channel::create_server(false).unwrap();
                    std::mem::replace(&mut listener,next)
                };
                let request=vz_mcp::channel::read_frame(&mut stream,vz_mcp::REQUEST_CAP).await.unwrap();
                assert_eq!(serde_json::from_slice::<Value>(&request).unwrap()["client"],"a".repeat(32));
                if phase=="lock_wait" || phase=="after_tool" {
                    // The frame is received; the app pauses before releasing any result.
                    let _tool_result=(phase=="after_tool").then(||json!({"hits":[{"text":"private result"}]}));
                    client.write(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"test"}}));
                    let mut byte=[0];
                    assert_eq!(tokio::time::timeout(Duration::from_secs(1),stream.read(&mut byte)).await.unwrap().unwrap(),0);
                    // A later ping is a stdout barrier: no cancelled response precedes it.
                    client.request(id+1,"ping",json!({}));client.response(id+1);
                } else {
                    let response=if phase=="removed" {json!({"ok":false,"code":"access_denied","message":vz_mcp::NO_ACCESS})} else {json!({"ok":true,"result":{"hits":[{"text":"private result"}]}})};
                    vz_mcp::channel::write_frame(&mut stream,&serde_json::to_vec(&response).unwrap(),vz_mcp::RESPONSE_CAP).await.unwrap();
                    // Handoff has completed before removal: the bridge may still emit it.
                    let result=client.response(id);
                    assert_eq!(result["result"]["isError"],phase=="removed");
                    if phase!="removed" {assert_eq!(result["result"]["structuredContent"]["hits"][0]["text"],"private result");}
                }
            });
        }
    }
    drop(listener);
    // Valid clients also get the not-running text when no app owns the channel.
    for modern in [true, false] {
        let mut client = Client::new(home.path(), modern);
        if modern {
            client.request(1, "server/discover", json!({}));
        } else {
            client.request(1, "initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
        }
        client.response(1);
        if !modern {
            client.write(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        }
        for (id, tool, args) in [
            (2, "list_collections", json!({})),
            (3, "search", json!({"query":"test"})),
            (4, "get_document", json!({"doc_id":"d"})),
            (5, "get_chunk", json!({"chunk_id":"c"})),
        ] {
            client.request(id, "tools/call", json!({"name":tool,"arguments":args}));
            let response = client.response(id);
            assert_eq!(response["result"]["isError"], true);
            assert_eq!(
                response["result"]["content"][0]["text"],
                vz_mcp::NOT_RUNNING
            );
        }
    }
}
