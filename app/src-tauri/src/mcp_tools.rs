//! Read-only library tools. The caller owns the inference guard and has passed gate 2.
use crate::models::EngineCache;
use serde_json::{json, Value};
use std::path::Path;
use vz_core::{engine::Engines, storage::State};

#[allow(clippy::too_many_arguments)]
pub async fn call<E, F>(
    dir: &Path,
    root: &Path,
    operations: &crate::models::OperationState<E>,
    apps: &crate::mcp_apps::Apps,
    request: crate::mcp_channel::Request,
    cancel: tokio_util::sync::CancellationToken,
    execute: F,
) -> Value
where
    E: crate::models::CachedEngine + Send + 'static,
    F: FnOnce(&mut EngineCache<E>, crate::mcp_channel::Request) -> Result<Value, String>
        + Send
        + 'static,
{
    vz_pack::listing_policy::system().refresh();
    use crate::mcp_channel::gate;
    let failure = |code: &str, message: &str| json!({"ok":false,"code":code,"message":message});
    let checked = gate(dir, &request);
    let call_id = apps.record(dir, &request.client, &request.tool, checked.as_ref().err());
    if let Err(e) = checked {
        return e;
    }
    if cancel.is_cancelled() {
        return failure("cancelled", vz_mcp::NO_ACCESS);
    }
    if let Err(e) = vz_mcp::protocol::validate_args(&request.tool, &request.args) {
        return failure("invalid_args", e);
    }
    if crate::desktop_pipeline::require_index(root).is_err() {
        return failure(
            "no_library",
            "vectorAIz has no library yet. Add a folder in vectorAIz first.",
        );
    }
    let mut guard = tokio::select! {biased;
        _=cancel.cancelled()=>return failure("cancelled",vz_mcp::NO_ACCESS),
        result=tokio::time::timeout(std::time::Duration::from_secs(10),operations.wait_inference())=>match result {
            Ok(g)=>g,Err(_)=>return failure("busy","vectorAIz is busy indexing. Try again in a moment."),
        }
    };
    if let Err(e) = gate(dir, &request) {
        return e;
    }
    if cancel.is_cancelled() {
        return failure("cancelled", vz_mcp::NO_ACCESS);
    }
    let result = tokio::task::spawn_blocking(move || {
        if cancel.is_cancelled() {
            return Err(vz_mcp::NO_ACCESS.to_owned());
        }
        execute(&mut guard, request)
    })
    .await;
    #[cfg(test)]
    crate::mcp_channel::pause(dir, "after_tool").await;
    let response = match result {
        Ok(Ok(result)) => json!({"ok":true,"result":result}),
        Ok(Err(e)) => failure("tool_error", customer_error(&e)),
        Err(error) => {
            eprintln!("local library tool task failed: {error}");
            failure("tool_error", "The library could not be read.")
        }
    };
    apps.completed(dir, &call_id, &response);
    response
}

fn customer_error(detail: &str) -> &str {
    match detail {
        vz_mcp::NO_ACCESS | "That document isn't in your vectorAIz library." | "Unknown tool." => {
            detail
        }
        _ => {
            eprintln!("local library tool failed: {detail}");
            "The library could not be read."
        }
    }
}
#[cfg(test)]
pub static INDEX_OPENS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn open_index(root: &Path, engines: &Engines) -> anyhow::Result<State> {
    #[cfg(test)]
    INDEX_OPENS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(State::open_index(root, &engines.recipe)?.0)
}
pub fn execute(
    cache: &mut EngineCache<Engines>,
    root: &Path,
    assets: &Path,
    pins: &Path,
    cpu: bool,
    tool: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    vz_pack::listing_policy::system().refresh();
    let engines = cache.get_or_load(false, || Engines::load_embedding(assets, pins, cpu))?;
    let state = open_index(root, engines)?;
    let docs = state.documents()?;
    let name = |path: &str| path.rsplit(['/', '\\']).next().unwrap_or("").to_owned();
    match tool {
        "list_collections" => {
            let folder = vz_core::active_index::last_indexed_folder(root).unwrap_or_default();
            Ok(
                json!({"collections":[{"id":"library","name":name(&folder),"documents":docs.len()}]}),
            )
        }
        "search" => {
            let query = args["query"].as_str().unwrap();
            let hits = tokio::runtime::Runtime::new()?.block_on(state.search(
                query,
                &engines.vector(query, true)?,
                args["top_k"].as_u64().unwrap_or(8) as usize,
            ))?;
            let names = crate::desktop_pipeline::file_names(&state);
            Ok(
                json!({"hits":hits.into_iter().map(|h|json!({"chunk_id":h.chunk.chunk_id,"doc_id":h.chunk.doc_id,"file_name":names.get(&h.chunk.doc_id).cloned().unwrap_or_default(),"page":h.chunk.page,"text":h.chunk.text,"score":h.score})).collect::<Vec<_>>()}),
            )
        }
        "get_document" => {
            let doc = docs
                .iter()
                .find(|d| Some(d.doc_id.as_str()) == args["doc_id"].as_str())
                .ok_or_else(|| anyhow::anyhow!("That document isn't in your vectorAIz library."))?;
            let all = doc
                .chunks
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let text: String = all.chars().take(200000).collect();
            Ok(
                json!({"doc_id":doc.doc_id,"file_name":name(&doc.path),"page_count":doc.pages,"truncated":text.len()<all.len(),"text":text}),
            )
        }
        "get_chunk" => {
            for doc in &docs {
                if let Some(c) = doc
                    .chunks
                    .iter()
                    .find(|c| Some(c.chunk_id.as_str()) == args["chunk_id"].as_str())
                {
                    return Ok(
                        json!({"chunk_id":c.chunk_id,"doc_id":doc.doc_id,"file_name":name(&doc.path),"page":c.page,"text":c.text}),
                    );
                }
            }
            anyhow::bail!("That document isn't in your vectorAIz library.")
        }
        _ => anyhow::bail!("Unknown tool."),
    }
}

#[cfg(test)]
mod error_tests {
    #[test]
    fn engine_paths_stay_local() {
        for detail in [
            "open /private/library/index: permission denied",
            r"open C:\library\index: access denied",
        ] {
            let response = super::customer_error(detail);
            assert_eq!(response, "The library could not be read.");
            assert!(!response.contains(['/', '\\']));
        }
    }
}
