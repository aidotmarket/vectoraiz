//! Hand-written against schema/PIN.md. No data files or network transport.
use serde_json::{json, Value};
pub const MODERN: &str = "2026-07-28";
pub const LEGACY: &str = "2025-11-25";
pub const VERSIONS: [&str; 2] = [MODERN, LEGACY];
pub fn tools() -> Value {
    serde_json::from_str(include_str!("tools.json")).expect("bundled tool schemas")
}
pub fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
pub fn result(id: Value, mut result: Value, modern: bool) -> Value {
    if modern {
        result["resultType"] = json!("complete");
    }
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
pub fn tool_error(message: &str) -> Value {
    json!({"isError":true,"content":[{"type":"text","text":message}]})
}
pub fn validate_args(name: &str, args: &Value) -> Result<(), &'static str> {
    if !args.is_object() {
        return Err("Tool arguments must be an object.");
    }
    match name {
        "list_collections" => {}
        "search" => {
            if !args["query"]
                .as_str()
                .is_some_and(|s| (1..=2000).contains(&s.chars().count()))
            {
                return Err("query must contain 1 to 2,000 characters.");
            }
            if args.get("collection").is_some_and(|c| c != "library") {
                return Err("Unknown collection.");
            }
            if args
                .get("top_k")
                .is_some_and(|k| !k.as_u64().is_some_and(|k| (1..=20).contains(&k)))
            {
                return Err("top_k must be an integer from 1 to 20.");
            }
        }
        "get_document" if args["doc_id"].is_string() => {}
        "get_chunk" if args["chunk_id"].is_string() => {}
        "get_document" | "get_chunk" => return Err("An id is required."),
        _ => return Err("Unknown tool."),
    }
    Ok(())
}
/// None means a tools/call must be dispatched to the local channel.
pub fn immediate(request: &Value, legacy_ready: bool) -> (bool, Option<Value>) {
    let id = request["id"].clone();
    let method = request["method"].as_str().unwrap_or("");
    if request["jsonrpc"] != "2.0" || !request["id"].is_string() && !request["id"].is_number() {
        return (false, Some(error(id, -32600, "Invalid Request")));
    }
    if method == "initialize" {
        return (
            false,
            Some(result(
                id,
                json!({"protocolVersion":LEGACY,"capabilities":{"tools":{}},"serverInfo":{"name":"vectoraiz-mcp","version":env!("CARGO_PKG_VERSION")}}),
                false,
            )),
        );
    }
    let meta = &request["params"]["_meta"];
    let modern = meta
        .get("io.modelcontextprotocol/protocolVersion")
        .is_some()
        || !legacy_ready;
    if modern {
        let Some(version) = meta["io.modelcontextprotocol/protocolVersion"].as_str() else {
            return (true, Some(error(id, -32602, "Missing protocol version")));
        };
        if !meta["io.modelcontextprotocol/clientCapabilities"].is_object() {
            return (true, Some(error(id, -32602, "Missing client capabilities")));
        }
        if !VERSIONS.contains(&version) {
            let mut e = error(id, -32022, "Unsupported protocol version");
            e["error"]["data"] = json!({"requested":version,"supported":VERSIONS});
            return (true, Some(e));
        }
    }
    let r = match method {
        "server/discover" => {
            json!({"supportedVersions":VERSIONS,"capabilities":{"tools":{}},"cacheScope":"private","ttlMs":0,
            "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"vectoraiz-mcp","version":env!("CARGO_PKG_VERSION")}}})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":tools(),"cacheScope":"private","ttlMs":0}),
        "tools/call" => {
            if let Err(message) = validate_args(
                request["params"]["name"].as_str().unwrap_or(""),
                &request["params"]["arguments"],
            ) {
                return (modern, Some(error(id, -32602, message)));
            }
            return (modern, None);
        }
        _ => return (modern, Some(error(id, -32601, "Method not found"))),
    };
    (modern, Some(result(id, r, modern)))
}
