#![cfg(feature = "office-feasibility")]
//! Real SQLite/Lance storage, deterministic synthetic vectors. NOT native-model
//! capacity or full viewer qualification. Results are written only on opt-in.
use anyhow::{ensure, Result};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use vz_core::{
    pipeline::{chunk, sha256, Document, Page},
    storage::State,
};

fn output() -> Result<Option<PathBuf>> {
    let Some(path) = std::env::var_os("VZ_OFFICE_EVIDENCE") else {
        return Ok(None);
    };
    let path = PathBuf::from(path).canonicalize()?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .canonicalize()?;
    ensure!(
        !path.starts_with(repo),
        "private evidence must be outside repository"
    );
    Ok(Some(path))
}

async fn generation(
    root: &Path,
    count: usize,
    label: &str,
    engines: Option<&vz_core::engine::Engines>,
) -> Result<()> {
    let mut state = State::open(root)?;
    for i in 0..count {
        let id = format!("synthetic{i:05}");
        let text =
            format!("{id} {label} public synthetic office record. The reference number is {i}.");
        let pages = vec![Page {
            page: 1,
            text: text.clone(),
            needs_ocr: false,
        }];
        let mut chunks = chunk(&id, &pages);
        for c in &mut chunks {
            c.embedding = if let Some(engines) = engines {
                engines.vector(&c.text, false)?
            } else {
                let mut vector = vec![0.0; 32];
                vector[i % 32] = 1.0;
                vector
            };
        }
        let doc = Document {
            doc_id: id.clone(),
            path: format!("{id}.txt"),
            sha256: sha256(text.as_bytes()),
            bytes: text.len() as u64,
            pages: 1,
            kind: "txt".into(),
            needs_ocr_pages: vec![],
            chunks,
        };
        let staged = state.stage(&doc).await?;
        state.commit(&doc, "synthetic", &staged)?;
    }
    Ok(())
}

#[test]
fn office_feasibility_storage_and_25_searches() -> Result<()> {
    let started = Instant::now();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    let runtime_creation_ms = started.elapsed().as_secs_f64() * 1000.0;
    runtime.block_on(storage_and_searches(runtime_creation_ms))
}
async fn storage_and_searches(runtime_creation_ms: f64) -> Result<()> {
    let external = output()?;
    let temp = tempfile::tempdir()?;
    let root = external.as_deref().unwrap_or(temp.path());
    let run = tempfile::Builder::new()
        .prefix("storage-")
        .tempdir_in(root)?;
    let count = std::env::var("VZ_OFFICE_DOCUMENTS")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(32usize);
    ensure!(
        (25..=4096).contains(&count),
        "document count must be 25..4096"
    );
    let old = run.path().join("old");
    let engines = std::sync::Arc::new(
        match (
            std::env::var_os("VZ_OFFICE_ASSETS"),
            std::env::var_os("VZ_OFFICE_PINS"),
        ) {
            (Some(assets), Some(pins)) => Some(vz_core::engine::Engines::load_embedding(
                Path::new(&assets),
                Path::new(&pins),
                false,
            )?),
            _ => None,
        },
    );
    generation(&old, count, "oldgeneration", engines.as_ref().as_ref()).await?;
    let mut samples = Vec::new();
    let guard = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let state = State::open(&old)?;
    let snapshot_start = Instant::now();
    let snapshot = std::sync::Arc::new(
        vz_core::storage::office::Snapshot::build(&state, &run.path().join("snapshot")).await?,
    );
    let snapshot_build_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
    drop(state);
    // Both versions use the same actual native model, queries and oracle.
    // Baseline holds the inference guard through storage, matching prior C1.
    for implementation in ["baseline", "snapshot"] {
        if implementation == "baseline" && std::env::var_os("VZ_OFFICE_SKIP_BASELINE").is_some() {
            continue;
        }
        for (mode, people) in [("single", 1), ("cold_first_process_pass", 25), ("warm", 25)] {
            let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(people));
            let mut jobs = Vec::new();
            for person in 0..people {
                let barrier = barrier.clone();
                let guard = guard.clone();
                let old = old.clone();
                let engines = engines.clone();
                let snapshot = snapshot.clone();
                jobs.push(async move {
                    let start = Instant::now();
                    barrier.wait().await;
                    let lock_requested = Instant::now();
                    let inference = guard.lock().await;
                    let queue_ms = lock_requested.elapsed().as_secs_f64() * 1000.0;
                    let arrival_to_guard_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let state_start = Instant::now();
                    let (opened, state_open) = if implementation == "baseline" {
                        let (state, timing) = State::office_open_timed(&old)?;
                        (Some(state), timing)
                    } else {
                        (None, json!({}))
                    };
                    let state_open_ms = state_start.elapsed().as_secs_f64() * 1000.0;
                    let expected = format!("synthetic{person:05}");
                    let (vector, embedding) = if let Some(engines) = engines.as_ref() {
                        engines.office_vector_timed(&expected, true)?
                    } else {
                        let mut vector = vec![0.0; 32];
                        vector[person % 32] = 1.0;
                        (vector, json!({"total_ms": 0.0}))
                    };
                    let search_start = Instant::now();
                    let (hits, storage) = if let Some(opened) = opened {
                        let (hits, timing) =
                            opened.office_search_timed(&expected, &vector, 8).await?;
                        drop(inference);
                        (hits, serde_json::to_value(timing)?)
                    } else {
                        drop(inference);
                        let (hits, timing) = snapshot.search_timed(&expected, &vector, 8).await?;
                        (hits, serde_json::to_value(timing)?)
                    };
                    let correct = hits.iter().any(|hit| {
                        hit.chunk.doc_id == expected && hit.chunk.text.contains("oldgeneration")
                    });
                    Ok::<_, anyhow::Error>(json!({
                        "implementation":implementation, "identity":person, "mode":mode,
                        "correct":correct, "latency_ms":start.elapsed().as_secs_f64()*1000.0,
                        "queue_ms":queue_ms, "arrival_to_guard_ms":arrival_to_guard_ms,
                        "state_open_ms":state_open_ms, "state_open":state_open,
                        "embedding":embedding, "storage":storage,
                        "search_ms":search_start.elapsed().as_secs_f64()*1000.0,
                        "per_query_tokio_runtime_creation_ms":0
                    }))
                });
            }
            for result in futures::future::join_all(jobs).await {
                samples.push(result?);
            }
            // Preserve partial results even if a later continuity probe fails.
            if let Some(path) = &external {
                std::fs::write(
                    path.join("search-partial.json"),
                    serde_json::to_vec_pretty(&samples)?,
                )?;
            }
        }
    }
    let next = run.path().join("next");
    // Separate roots are essential even with an identical recipe. Old State
    // remains searchable while the next writer owns its exclusive lock.
    let building = generation(&next, 25, "newgeneration", engines.as_ref().as_ref());
    let old_during_build = async {
        let old_state = State::open(&old)?;
        let vector = if let Some(engines) = engines.as_ref() {
            engines.vector("oldgeneration", true)?
        } else {
            vec![1.0; 32]
        };
        for _ in 0..3 {
            let hits = old_state.search("oldgeneration", &vector, 8).await?;
            ensure!(
                !hits.is_empty() && hits.iter().all(|h| h.chunk.text.contains("oldgeneration")),
                "old generation unavailable during build"
            );
        }
        Ok::<_, anyhow::Error>(())
    };
    let (built, searched) = tokio::join!(building, old_during_build);
    built?;
    searched?;
    let writing = State::open(&next)?;
    assert!(State::open(&next).is_err());
    let old_state = State::open(&old)?;
    let continuity_vector = if let Some(engines) = engines.as_ref() {
        engines.vector("oldgeneration", true)?
    } else {
        vec![1.0; 32]
    };
    assert!(old_state
        .search("oldgeneration", &continuity_vector, 8)
        .await?
        .iter()
        .all(|h| h.chunk.text.contains("oldgeneration")));
    assert!(State::open(&old).is_err(), "desktop index-locked busy seam");
    // Snapshot readers do not acquire State's exclusive writer lock.
    let snapshot_hits = snapshot
        .search("oldgeneration", &continuity_vector, 8)
        .await?;
    ensure!(
        !snapshot_hits.is_empty()
            && snapshot_hits
                .iter()
                .all(|h| h.chunk.text.contains("oldgeneration")),
        "snapshot reader blocked by source writer lock"
    );
    drop(old_state);
    drop(writing);
    let pointer = run.path().join("office-generation.json");
    std::fs::write(&pointer, b"old")?;
    std::fs::write(run.path().join("pending"), b"next")?;
    // Race selection readers against atomic pointer replacement. They may see
    // exactly the old or new complete pointer, never an empty/partial value.
    let reader_pointer = pointer.clone();
    let reader = std::thread::spawn(move || -> Result<()> {
        for _ in 0..1000 {
            let value = std::fs::read_to_string(&reader_pointer)?;
            ensure!(
                value == "old" || value == "next",
                "partial promotion pointer"
            );
        }
        Ok(())
    });
    std::fs::rename(run.path().join("pending"), &pointer)?;
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("pointer reader panicked"))??;
    assert_eq!(std::fs::read_to_string(&pointer)?, "next");
    let selected = State::open(&run.path().join(std::fs::read_to_string(pointer)?))?;
    let hits = selected
        .search("newgeneration", &continuity_vector, 8)
        .await?;
    ensure!(
        !hits.is_empty() && hits.iter().all(|h| h.chunk.text.contains("newgeneration")),
        "new generation missing after promotion"
    );
    // A reader that captured the old Arc before promotion retains old data.
    let old_hits = snapshot
        .search("oldgeneration", &continuity_vector, 8)
        .await?;
    ensure!(
        !old_hits.is_empty()
            && old_hits
                .iter()
                .all(|h| h.chunk.text.contains("oldgeneration")),
        "old snapshot changed after promotion"
    );
    let promoted_snapshot =
        vz_core::storage::office::Snapshot::build(&selected, &run.path().join("promoted-snapshot"))
            .await?;
    let new_hits = promoted_snapshot
        .search("newgeneration", &continuity_vector, 8)
        .await?;
    ensure!(
        !new_hits.is_empty()
            && new_hits
                .iter()
                .all(|h| h.chunk.text.contains("newgeneration")),
        "promoted snapshot mixed generations"
    );
    let value = json!({"kind":if engines.is_some() {"native_embedding_real_storage"} else {"real_storage_synthetic_vectors"}, "documents":count,"chunks":count,"samples":samples,"snapshot_build_ms":snapshot_build_ms,"runtime_creation_ms":runtime_creation_ms,"snapshot_read_with_writer_lock":true,"old_snapshot_retained_after_promotion":true,"separate_generation_and_locked_busy":true,"promotion":"test-local atomic rename; not active_index::reindex", "cold_limit":"single then first 25-way pass after indexing/snapshot preparation; OS disk cache not flushed", "concurrency":"25 futures on one coordinator task, shared four-worker Tokio runtime; synchronous embeddings serialize polling; latency includes arrival barrier and scheduling", "native_model":engines.is_some(), "recipe":engines.as_ref().as_ref().map(|x| &x.recipe)});
    if let Some(path) = external {
        std::fs::write(
            path.join("storage.json"),
            serde_json::to_vec_pretty(&value)?,
        )?;
    }
    // Timing/correctness failures remain raw evidence rather than changing the
    // contracts or treating busy as a successful search.
    println!("C1 storage samples: {}", value);
    Ok(())
}

#[test]
fn office_feasibility_native_abort() -> Result<()> {
    use std::sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    };
    let (Some(assets), Some(pins)) = (
        std::env::var_os("VZ_OFFICE_ASSETS"),
        std::env::var_os("VZ_OFFICE_PINS"),
    ) else {
        println!("UNRUN native abort: VZ_OFFICE_ASSETS/VZ_OFFICE_PINS required");
        return Ok(());
    };
    let model = Arc::new(Mutex::new(vz_core::engine::ChatOnly::load(
        Path::new(&assets),
        Path::new(&pins),
        false,
    )?));
    let mut rows = Vec::new();
    for target in [1, 2] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(AtomicU8::new(0));
        let model_job = model.clone();
        let cancel_job = cancelled.clone();
        let phase_job = phase.clone();
        let worker = std::thread::spawn(move || {
            let guard = model_job.lock().unwrap();
            guard.office_abort_probe(&cancel_job, &phase_job)
        });
        let start = Instant::now();
        while phase.load(Ordering::Acquire) < target
            && !worker.is_finished()
            && start.elapsed().as_secs() < 30
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        let signalled_phase = phase.load(Ordering::Acquire);
        let signalled = Instant::now();
        cancelled.store(true, Ordering::Release);
        let aborted = worker
            .join()
            .map_err(|_| anyhow::anyhow!("native probe panicked"))??;
        let released = model.try_lock().is_ok();
        let ms = signalled.elapsed().as_secs_f64() * 1000.0;
        rows.push(json!({"phase":if target==1 {"prefill"} else {"decode"}, "signalled_phase":signalled_phase,"aborted":aborted,"guard_released":released,"release_ms":ms,"status":if aborted && released && ms<=1000.0 && signalled_phase == target {"pass"} else {"fail"}}));
    }
    if let Some(root) = output()? {
        std::fs::write(
            root.join("native-abort.json"),
            serde_json::to_vec_pretty(&rows)?,
        )?;
    }
    println!("native abort measurements: {}", json!(rows));
    Ok(())
}

#[test]
fn office_feasibility_native_contention() -> Result<()> {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    let (Some(assets), Some(pins)) = (
        std::env::var_os("VZ_OFFICE_ASSETS"),
        std::env::var_os("VZ_OFFICE_PINS"),
    ) else {
        println!("UNRUN native refusal: explicit pinned models required");
        return Ok(());
    };
    let engines = Arc::new(Mutex::new(vz_core::engine::Engines::load(
        Path::new(&assets),
        Path::new(&pins),
        false,
    )?));
    let mut rows = serde_json::Map::new();
    for mode in [
        "desktop_indexing",
        "office_refresh",
        "desktop_ask",
        "combined",
    ] {
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        let worker_engines = engines.clone();
        let worker = std::thread::spawn(move || -> Result<()> {
            let result = (|| {
                if mode != "desktop_ask" {
                    for _ in 0..2 {
                        {
                            let engine = worker_engines.lock().unwrap();
                            for i in 0..32 {
                                engine.vector(
                                    &format!(
                                        "Synthetic public reference {i} office budget record."
                                    ),
                                    false,
                                )?;
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
                if mode == "desktop_ask" || mode == "combined" {
                    let engine = worker_engines.lock().unwrap();
                    engine.stream(
                        "What is the reference?",
                        "[1] The reference is 1927.",
                        |_| {},
                    )?;
                }
                Ok(())
            })();
            done.store(true, Ordering::Release);
            result
        });
        let start = Instant::now();
        let mut refusal = None;
        let mut maximum = 0.0_f64;
        let mut attempts = Vec::new();
        while !finished.load(Ordering::Acquire) {
            let now = Instant::now();
            let admitted = engines.try_lock().is_ok();
            if admitted {
                if let Some(since) = refusal.take() {
                    maximum = maximum.max(now.duration_since(since).as_secs_f64() * 1000.0);
                }
            } else {
                refusal.get_or_insert(now);
            }
            attempts
                .push(json!({"at_ms":start.elapsed().as_secs_f64()*1000.0,"admitted":admitted}));
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("contention worker panicked"))??;
        if let Some(since) = refusal {
            maximum = maximum.max(since.elapsed().as_secs_f64() * 1000.0);
        }
        rows.insert(mode.into(), json!({"measured_max_ms":maximum,"accepted_max_ms":null,"status":"blocked","reason":"native inference guard on synthetic 64-chunk workload; proposed batch yielding, not product indexing/Refresh qualification", "sampling_ms":1,"attempts":attempts}));
    }
    if let Some(root) = output()? {
        std::fs::write(root.join("refusal.json"), serde_json::to_vec_pretty(&rows)?)?;
    }
    println!("native contention scenarios complete; raw refusal traces retained");
    Ok(())
}

#[test]
fn office_feasibility_runtime_cost() -> Result<()> {
    // Desktop/MCP construct a runtime per request; the original C1 harness
    // does not. Measure this separately rather than attributing invented cost.
    let mut samples = Vec::new();
    for _ in 0..25 {
        let started = Instant::now();
        let runtime = tokio::runtime::Runtime::new()?;
        let creation_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        drop(runtime);
        samples.push(
            json!({"creation_ms":creation_ms,"teardown_ms":started.elapsed().as_secs_f64()*1000.0}),
        );
    }
    if let Some(root) = output()? {
        std::fs::write(
            root.join("runtime.json"),
            serde_json::to_vec_pretty(&samples)?,
        )?;
    }
    Ok(())
}
