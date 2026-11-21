//! Hand-written against schema/PIN.md. No data files or network transport.
use serde_json::{json, Value};
pub const LISTING_HANDOFF: &str = "get_listing_handoff";
pub const LISTING_HANDOFF_AVAILABILITY: &str = "listing_handoff_availability";
pub const MODERN: &str = "2026-07-28";
pub const LEGACY: &str = "2025-11-25";
pub const VERSIONS: [&str; 2] = [MODERN, LEGACY];
pub fn tools() -> Value {
    tools_for_preview(false)
}

pub fn tools_for_preview(handoff_available: bool) -> Value {
    let mut tools: Vec<Value> =
        serde_json::from_str(include_str!("tools.json")).expect("bundled tool schemas");
    if !handoff_available {
        tools.retain(|tool| tool["name"] != LISTING_HANDOFF);
    }
    json!(tools)
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
/// Only absence means an empty object; explicit null and other types stay invalid.
pub fn normalize_arguments(request: &mut Value) {
    if request["method"] == "tools/call" {
        if let Some(params) = request["params"].as_object_mut() {
            params.entry("arguments").or_insert_with(|| json!({}));
        }
    }
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
        LISTING_HANDOFF if args["current"].is_object() && args["request"].is_object() => {}
        LISTING_HANDOFF => return Err("A current review and handoff request are required."),
        _ => return Err("Unknown tool."),
    }
    Ok(())
}
/// None means a tools/call must be dispatched to the local channel.
pub fn immediate(request: &Value, legacy_ready: bool) -> (bool, Option<Value>) {
    immediate_with_preview(request, legacy_ready, || {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        if runtime.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return false;
        }
        // The stdio bridge and desktop are different processes. Ask the same-user
        // desktop channel, with a short deadline, for its process-local flag.
        tokio::task::block_in_place(|| runtime.block_on(desktop_handoff_available()))
    })
}

async fn desktop_handoff_available() -> bool {
    let request = json!({"v":1,"client":"","tool":LISTING_HANDOFF_AVAILABILITY,"args":{}});
    let Ok(Ok(bytes)) = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        crate::channel::call(request.to_string().as_bytes()),
    )
    .await
    else {
        return false;
    };
    serde_json::from_slice::<Value>(&bytes)
        .is_ok_and(|response| response["listing_handoff_available"] == true)
}

fn immediate_with_preview(
    request: &Value,
    legacy_ready: bool,
    handoff_available: impl FnOnce() -> bool,
) -> (bool, Option<Value>) {
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
        "tools/list" => {
            json!({"tools":tools_for_preview(handoff_available()),"cacheScope":"private","ttlMs":0})
        }
        "tools/call" => {
            if request["params"]["name"] == LISTING_HANDOFF && !handoff_available() {
                return (modern, Some(error(id, -32602, "Unknown tool.")));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_handoff_discovery_and_dispatch_follow_preview() {
        let list = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        let call = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":LISTING_HANDOFF,"arguments":{"current":{},"request":{}}
        }});
        for enabled in [false, true] {
            let result = immediate_with_preview(&list, true, || enabled).1.unwrap();
            let tools = result["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), if enabled { 5 } else { 4 });
            let handoff = tools.iter().find(|tool| tool["name"] == LISTING_HANDOFF);
            assert_eq!(handoff.is_some(), enabled);
            if let Some(handoff) = handoff {
                assert_eq!(handoff["annotations"]["readOnlyHint"], false);
            }
            assert_eq!(
                immediate_with_preview(&call, true, || enabled).1.is_none(),
                enabled
            );
        }
    }
}
