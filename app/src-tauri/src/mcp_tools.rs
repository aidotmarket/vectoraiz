//! Local library tools and their activity records.
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

// The build-generated app manifest covers the regular commands. Add the
// preview command at runtime, with the invoke handler enforcing the main window.
pub fn allow_listing_handoff<R: tauri::Runtime>(
    context: &mut tauri::Context<R>,
    preview_enabled: bool,
) {
    if preview_enabled {
        context.runtime_authority_mut().__allow_command(
            vz_mcp::protocol::LISTING_HANDOFF.into(),
            tauri::utils::acl::ExecutionContext::Local,
        );
    }
}

pub fn listing_handoff_visible(preview_enabled: bool, window: &str) -> bool {
    preview_enabled && window == "main"
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingArguments {
    pub current: vz_pack::review_store::CurrentReview,
    pub request: vz_pack::handoff::HandoffRequest,
}

pub async fn call_listing<F>(
    dir: &Path,
    apps: &crate::mcp_apps::Apps,
    request: crate::mcp_channel::Request,
    cancel: tokio_util::sync::CancellationToken,
    require_preview: impl Fn() -> Result<(), vz_pack::listing_policy::ListingError>,
    execute: impl FnOnce(ListingArguments, tokio_util::sync::CancellationToken) -> F,
) -> Value
where
    F: std::future::Future<
        Output = Result<
            vz_pack::handoff::ListingHandoff,
            vz_pack::listing_commands::ListingFailure,
        >,
    >,
{
    use crate::mcp_channel::gate;
    let policy_failure = |error: vz_pack::listing_policy::ListingError| json!({"ok":false,"code":error,"message":error.to_string()});
    let checked = require_preview()
        .map_err(policy_failure)
        .and_then(|()| gate(dir, &request));
    let call_id = apps.record(dir, &request.client, &request.tool, checked.as_ref().err());
    let response = async {
        if let Err(error) = checked {
            return error;
        }
        let arguments = match serde_json::from_value(request.args.clone()) {
            Ok(arguments) => arguments,
            Err(_) => {
                return json!({"ok":false,"code":"invalid_args","message":"Invalid listing handoff request."})
            }
        };
        let worker_cancel = cancel.child_token();
        let _cancel_on_drop = worker_cancel.clone().drop_guard();
        let result = tokio::select! { biased;
            _ = cancel.cancelled() => return json!({"ok":false,"code":"cancelled","message":vz_mcp::NO_ACCESS}),
            result = execute(arguments, worker_cancel) => result,
        };
        if let Err(error) = require_preview() {
            return policy_failure(error);
        }
        if let Err(error) = gate(dir, &request) {
            return error;
        }
        match result {
            Ok(result) => json!({"ok":true,"result":result}),
            Err(error) => policy_failure(error.code),
        }
    }
    .await;
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
    let name = |path: &str| path.rsplit(['/', '\\']).next().unwrap_or("").to_owned();
    match tool {
        "list_collections" => {
            let folder = vz_core::active_index::last_indexed_folder(root).unwrap_or_default();
            Ok(
                json!({"collections":[{"id":"library","name":name(&folder),"documents":state.documents()?.len()}]}),
            )
        }
        "search" => {
            let query = args["query"].as_str().unwrap();
            let report =
                tokio::runtime::Runtime::new()?.block_on(state.search_with_availability(
                    query,
                    &engines.vector(query, true)?,
                    args["top_k"].as_u64().unwrap_or(8) as usize,
                ))?;
            let hits: Vec<_> = report
                .hits
                .into_iter()
                .map(|hit| {
                    let chunk = hit.chunk;
                    let file_name = report.paths.get(&chunk.doc_id).map(|path| name(path));
                    json!({
                        "chunk_id": chunk.chunk_id,
                        "doc_id": chunk.doc_id,
                        "file_name": file_name.unwrap_or_default(),
                        "page": chunk.page,
                        "text": chunk.text,
                        "score": hit.score,
                    })
                })
                .collect();
            let unavailable: Vec<_> = report
                .unavailable
                .iter()
                .map(|item| name(&item.path))
                .collect();
            Ok(json!({"hits": hits, "unavailable": unavailable}))
        }
        "get_document" => {
            let docs = state.documents()?;
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
            for doc in state.documents()? {
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
    fn desktop_handoff_is_absent_by_default_and_confined_to_local_main_window() {
        use tauri::ipc::Origin;
        use vz_mcp::protocol::LISTING_HANDOFF;
        let mut context: tauri::Context<tauri::test::MockRuntime> =
            tauri::test::mock_context(tauri::test::noop_assets());
        super::allow_listing_handoff(&mut context, false);
        let local = Origin::Local;
        assert!(context
            .runtime_authority_mut()
            .resolve_access(LISTING_HANDOFF, "main", "main", &local)
            .is_none());
        assert!(!super::listing_handoff_visible(false, "main"));

        super::allow_listing_handoff(&mut context, true);
        let authority = context.runtime_authority_mut();
        assert!(authority
            .resolve_access(LISTING_HANDOFF, "main", "main", &local)
            .is_some());
        let remote = Origin::Remote {
            url: "https://example.com".parse().unwrap(),
        };
        assert!(authority
            .resolve_access(LISTING_HANDOFF, "main", "main", &remote)
            .is_none());
        assert!(!super::listing_handoff_visible(true, "other"));
        assert!(super::listing_handoff_visible(true, "main"));
    }

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
