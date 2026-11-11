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
    let run = lease.read().await;
    let cancellation = operations.start();
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
            let cancelled =
                (name == "Operations::cancel").then(|| operations.office_cancel_generation());
            let _mutation = lease.write().await;
            // Representative cache mutation seams; real UI call sites remain
            // unqualified. In particular cancel needs this separate lease too.
            if name == "Operations::cancel" {
                assert!(operations.office_invalidate_generation(cancelled.unwrap()));
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
    let lease = Arc::new(RwLock::new(()));
    let run = lease.read().await;
    let cancellation = operations.start();
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
        let generation = cancel_operations.office_cancel_generation();
        let _mutation = cancel_lease.write().await;
        assert!(cancel_operations.office_invalidate_generation(generation));
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

// Race each mutator at every boundary, including the yielded interval and drain.
// Tokio's FIFO, write-preferring RwLock prevents successor runs starving mutation.
#[tokio::test]
async fn office_feasibility_lease_boundaries_and_successor() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut rows = Vec::new();
    for mutation in [
        "delete",
        "activation",
        "download_promotion",
        "import_promotion",
        "Operations::cancel",
        "app_exit",
    ] {
        for boundary in 0..4 {
            let operations = Arc::new(OperationState::<Engine>::default());
            let lease = Arc::new(RwLock::new(()));
            let run = lease.clone().read_owned().await;
            let cancellation = operations.start();
            let mut mutation_task = None;
            let completed = Arc::new(AtomicBool::new(false));
            for batch in 0..4 {
                let mut cache = operations.wait_inference().await;
                assert_eq!(
                    cache.get_or_load(false, || Ok(Engine(1927))).unwrap().0,
                    1927
                );
                if batch == boundary {
                    let op = operations.clone();
                    let lock = lease.clone();
                    let done = completed.clone();
                    mutation_task = Some(tokio::spawn(async move {
                        let cancelled = matches!(mutation, "Operations::cancel" | "app_exit")
                            .then(|| op.office_cancel_generation());
                        let _exclusive = lock.write().await;
                        if let Some(generation) = cancelled {
                            assert!(op.office_invalidate_generation(generation));
                            // An obsolete deferred invalidation is harmless.
                            assert!(!op.office_invalidate_generation(generation));
                        } else {
                            drop(op.acquire_mutation().unwrap());
                        }
                        done.store(true, Ordering::Release);
                    }));
                    tokio::task::yield_now().await;
                    if matches!(mutation, "Operations::cancel" | "app_exit") {
                        assert!(
                            *cancellation.borrow(),
                            "signal must not wait for either lock"
                        );
                    }
                }
                drop(cache);
                tokio::task::yield_now().await;
                assert!(!completed.load(Ordering::Acquire));
            }
            let next_lock = lease.clone();
            let next_op = operations.clone();
            let done = completed.clone();
            let successor = tokio::spawn(async move {
                let _run = next_lock.read().await;
                assert!(done.load(Ordering::Acquire), "pending writer was starved");
                let cancellation = next_op.start();
                let mut cache = next_op.wait_inference().await;
                assert_eq!(
                    cache.get_or_load(false, || Ok(Engine(1928))).unwrap().0,
                    1928
                );
                assert!(!*cancellation.borrow(), "deferred cancel reached successor");
            });
            tokio::task::yield_now().await;
            assert!(!successor.is_finished());
            drop(run);
            mutation_task.unwrap().await.unwrap();
            successor.await.unwrap();
            rows.push(json!({"mutation":mutation,"boundary":boundary,"pinned_through_drain":true,"signal_immediate":true,"writer_before_successor":true,"successor_not_cancelled":true}));
        }
    }
    write(
        "lease-boundaries.json",
        json!({"status":"pass","trials":rows,"product_call_sites_integrated":false}),
    );
}

#[tokio::test]
async fn office_feasibility_real_mutation_seams() -> anyhow::Result<()> {
    use crate::models::{self, Pin, Source};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (Some(assets), Some(pins)) = (
        std::env::var_os("VZ_OFFICE_ASSETS"),
        std::env::var_os("VZ_OFFICE_PINS"),
    ) else {
        println!("UNRUN real mutation seams: native assets required");
        return Ok(());
    };
    let mut rows = Vec::new();
    for mutation in [
        "delete",
        "activation",
        "download_promotion",
        "import_promotion",
    ] {
        let operations = Arc::new(OperationState::<vz_core::engine::Engines>::default());
        let lease = Arc::new(RwLock::new(()));
        let run = lease.read().await;
        let _cancel = operations.start();
        let mut cache = operations.wait_inference().await;
        cache.get_or_load(false, || {
            vz_core::engine::Engines::load_embedding(
                std::path::Path::new(&assets),
                std::path::Path::new(&pins),
                false,
            )
        })?;
        drop(cache);
        let root = tempfile::tempdir()?;
        let candidate = root.path().join("candidate.gguf");
        let source = root.path().join("source.gguf");
        let content = b"C1 synthetic verified promotion candidate";
        std::fs::write(&candidate, b"previous candidate")?;
        std::fs::write(&source, content)?;
        let lock = lease.clone();
        let op = operations.clone();
        let directory = root.path().to_owned();
        let mutator = tokio::spawn(async move {
            let _exclusive = lock.write().await;
            match mutation {
                "delete" => {
                    let _cache = op.wait_inference().await;
                    std::fs::remove_file(directory.join("candidate.gguf"))?;
                }
                "activation" => {
                    op.activate_answer(
                        op.start_transfer(),
                        true,
                        |cache| cache.set_chat(None).map_err(|e| e.to_string()),
                        || {},
                    )
                    .await
                    .map_err(anyhow::Error::msg)?;
                }
                _ => {
                    let server = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                    let address = server.local_addr()?;
                    let pin = Pin {
                        file: "candidate.gguf".into(),
                        size: content.len() as u64,
                        sha256: vz_core::pipeline::sha256(content),
                        metadata: json!({"url":format!("http://{address}/candidate.gguf")}),
                    };
                    let request = tokio::spawn(async move {
                        if mutation == "download_promotion" {
                            let (mut connection, _) = server.accept().await?;
                            let mut buffer = [0; 4096];
                            connection.read(&mut buffer).await?;
                            connection.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",content.len()).as_bytes()).await?;
                            connection.write_all(content).await?;
                        }
                        Ok::<_, anyhow::Error>(())
                    });
                    let client = models::client()?;
                    let source = if mutation == "download_promotion" {
                        Source::Download(&client)
                    } else {
                        Source::Import(std::fs::File::open(directory.join("source.gguf"))?)
                    };
                    models::stage_verify_promote_guarded(
                        &directory,
                        &pin,
                        source,
                        op.start_transfer(),
                        &mut |_| {},
                        Some(op.promotion_lock()),
                    )
                    .await?;
                    request.await??;
                    assert_eq!(std::fs::read(directory.join("candidate.gguf"))?, content);
                }
            }
            Ok::<_, anyhow::Error>(())
        });
        let mut batches = Vec::new();
        for batch in 0..3 {
            let mut cache = operations.wait_inference().await;
            let engine =
                cache.get_or_load(false, || panic!("native engine changed during full run"))?;
            let vector = engine.vector("synthetic reference 1927", true)?;
            batches.push(json!({"batch":batch,"dimensions":vector.len()}));
            drop(cache);
            tokio::task::yield_now().await;
            assert!(!mutator.is_finished());
            assert_eq!(std::fs::read(&candidate)?, b"previous candidate");
        }
        drop(run);
        mutator.await??;
        rows.push(json!({"mutation":mutation,"native_engine_pinned":true,"batches":batches,"ran_after_drain":true}));
    }
    write(
        "real-mutation-seams.json",
        json!({"status":"pass","trials":rows,"limits":"real activation and verified transfer/cache seams wrapped in harness lease; delete filesystem seam; synthetic candidate bytes; product UI call sites remain unintegrated"}),
    );
    Ok(())
}
