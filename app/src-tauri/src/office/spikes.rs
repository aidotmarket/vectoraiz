//! C1 only. Proposed full-run lease/admission experiment, not product scheduling.
//! Uses the actual Operations cache with a deliberately synthetic engine. Native
//! prefill/decode cancellation and real model throughput need separate evidence.
use crate::models::{CachedEngine, OperationState};
use serde_json::json;
use std::{sync::Arc, time::Instant};
use tokio::sync::RwLock;

struct Engine(usize);
impl CachedEngine for Engine {
    fn ensure_chat(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn office_feasibility_run_lease_and_admission() {
    let operations = Arc::new(OperationState::<Engine>::default());
    // A distinct lease must cover ALL mutations, including cancel, while the
    // inference mutex is released. This wrapper is deliberately test-local.
    let lease = Arc::new(RwLock::new(()));
    let cancellation = operations.start();
    let run = lease.read().await;
    let mut trace = Vec::new();
    let start = Instant::now();
    let mutations = [
        "delete",
        "activation",
        "download_promotion",
        "import_promotion",
        "Operations::cancel",
    ];
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut mutators = Vec::new();
    for name in mutations {
        let lease = lease.clone();
        let operations = operations.clone();
        let completed = completed.clone();
        mutators.push(tokio::spawn(async move {
            if name == "Operations::cancel" {
                // Cancellation delivery is immediate; generation invalidation
                // waits for the complete run, not just an inference batch.
                operations.office_signal_cancel();
            }
            let _mutation = lease.write().await;
            // Representative cache mutation seams; real UI call sites remain
            // unqualified. In particular cancel needs this separate lease too.
            if name == "Operations::cancel" {
                operations.cancel();
            } else {
                drop(operations.acquire_mutation().unwrap());
            }
            completed.fetch_add(1, std::sync::atomic::Ordering::Release);
            name
        }));
    }
    for batch in 0..3 {
        {
            let mut cache = operations.acquire().unwrap();
            assert_eq!(
                cache.get_or_load(false, || Ok(Engine(1927))).unwrap().0,
                1927
            );
            trace.push(json!({"batch":batch,"event":"embed","chunks":32}));
        }
        for mutation in mutations {
            assert!(lease.try_write().is_err());
            trace.push(json!({"batch":batch,"event":"excluded","mutation":mutation}));
        }
        // Enqueue office FIRST, then test that waiting desktop/read-only MCP
        // actually win admission. Each actor waits for a grant, takes the real
        // inference guard and acknowledges completion before the next grant.
        let (arrivals, mut queue) = tokio::sync::mpsc::channel(6);
        let mut readers = Vec::new();
        for reader in [
            "office_search",
            "desktop_search",
            "mcp_search",
            "mcp_list",
            "mcp_document",
            "mcp_chunk",
        ] {
            let arrivals = arrivals.clone();
            let operations = operations.clone();
            readers.push(tokio::spawn(async move {
                let (grant, wait) = tokio::sync::oneshot::channel();
                let (done, completion) = tokio::sync::oneshot::channel();
                arrivals.send((reader, grant, completion)).await.unwrap();
                wait.await.unwrap();
                let mut cache = operations.wait_inference().await;
                assert_eq!(
                    cache
                        .get_or_load(false, || panic!("engine replaced during run"))
                        .unwrap()
                        .0,
                    1927
                );
                drop(cache);
                done.send(()).unwrap();
            }));
        }
        let mut pending = Vec::new();
        for _ in 0..6 {
            pending.push(queue.recv().await.unwrap());
        }
        pending.sort_by_key(|(reader, _, _)| *reader == "office_search");
        let order: Vec<_> = pending.iter().map(|(reader, _, _)| *reader).collect();
        assert_eq!(order.last(), Some(&"office_search"));
        for (reader, grant, done) in pending {
            grant.send(()).unwrap();
            done.await.unwrap();
            trace.push(json!({"batch":batch,"event":"admit","reader":reader}));
        }
        for reader in readers {
            reader.await.unwrap();
        }
        assert_eq!(completed.load(std::sync::atomic::Ordering::Acquire), 0);
        assert!(*cancellation.borrow(), "abort signal blocked by run lease");
    }
    drop(run);
    for mutator in mutators {
        trace.push(json!({"event":"mutation_after_run","mutation":mutator.await.unwrap()}));
    }
    assert_eq!(completed.load(std::sync::atomic::Ordering::Acquire), 5);
    let mutation = lease.write().await;
    operations.cancel();
    let mut cache = operations.acquire().unwrap();
    assert_eq!(
        cache.get_or_load(false, || Ok(Engine(1928))).unwrap().0,
        1928
    );
    drop(cache);
    drop(mutation);
    write(
        "operations.json",
        json!({"kind":"synthetic_engine_proposed_lease", "cancel_signal_during_run":true,"generation_invalidation_after_run":true, "elapsed_ms":start.elapsed().as_secs_f64()*1000.0,"trace":trace}),
    );
}

#[tokio::test]
async fn office_feasibility_current_cancel_gap() {
    let operations = OperationState::<Engine>::default();
    let mut cache = operations.acquire().unwrap();
    cache.get_or_load(false, || Ok(Engine(1))).unwrap();
    operations.cancel();
    drop(cache);
    let mut cache = operations.wait_inference().await;
    let replaced = cache.get_or_load(false, || Ok(Engine(2))).unwrap().0 == 2;
    assert!(
        replaced,
        "revalidate baseline if cancellation implementation changes"
    );
    write(
        "cancel-gap.json",
        json!({"status":"fail", "contract":"full-run generation exclusion in current Operations", "engine_replaced":replaced}),
    );
}

fn write(name: &str, value: serde_json::Value) {
    if let Some(root) = std::env::var_os("VZ_OFFICE_EVIDENCE") {
        let root = std::path::PathBuf::from(root).canonicalize().unwrap();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .canonicalize()
            .unwrap();
        assert!(
            !root.starts_with(repo),
            "evidence must be outside repository"
        );
        std::fs::write(root.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn office_feasibility_native_deferred_invalidation() -> anyhow::Result<()> {
    let (Some(assets), Some(pins)) = (
        std::env::var_os("VZ_OFFICE_ASSETS"),
        std::env::var_os("VZ_OFFICE_PINS"),
    ) else {
        println!("UNRUN native cache lease: explicit pinned models required");
        return Ok(());
    };
    let operations = Arc::new(OperationState::<vz_core::engine::Engines>::default());
    let cancellation = operations.start();
    let lease = Arc::new(RwLock::new(()));
    let run = lease.read().await;
    let mut cache = operations.acquire().unwrap();
    cache.get_or_load(false, || {
        vz_core::engine::Engines::load_embedding(
            std::path::Path::new(&assets),
            std::path::Path::new(&pins),
            false,
        )
    })?;
    drop(cache);
    let cancel_operations = operations.clone();
    let cancel_lease = lease.clone();
    let cancel = tokio::spawn(async move {
        cancel_operations.office_signal_cancel();
        let _mutation = cancel_lease.write().await;
        cancel_operations.cancel();
    });
    tokio::task::yield_now().await;
    assert!(*cancellation.borrow());
    let mut rows = Vec::new();
    for batch in 0..3 {
        let mut cache = operations.wait_inference().await;
        let engine =
            cache.get_or_load(false, || panic!("native engine replaced between batches"))?;
        let vector = engine.vector("synthetic reference 1927", true)?;
        rows.push(json!({"batch":batch,"dimensions":vector.len(),"abort_signalled":*cancellation.borrow(),"generation_pinned":true}));
        drop(cache);
        tokio::task::yield_now().await;
        assert!(
            !cancel.is_finished(),
            "generation mutator ran before lease release"
        );
    }
    drop(run);
    cancel.await?;
    let mut reloaded = false;
    let mut cache = operations.wait_inference().await;
    cache.get_or_load(false, || {
        reloaded = true;
        vz_core::engine::Engines::load_embedding(
            std::path::Path::new(&assets),
            std::path::Path::new(&pins),
            false,
        )
    })?;
    assert!(reloaded);
    write(
        "native-cache-lease.json",
        json!({"status":"pass","kind":"native_engine_test_local_lease_protocol","batches":rows,"reloaded_after_run":reloaded,"product_call_sites_integrated":false}),
    );
    Ok(())
}
