#![cfg(feature = "office-feasibility")]
//! Private, explicit opt-in rerun; ordinary cargo test does not run a corpus job.
use anyhow::{Context, Result};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Instant};
use vz_core::storage::office_s1934::{self as harness, Candidate, Snapshot};

fn rss() -> u64 {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct Info {
            virtual_size: u64,
            resident_size: u64,
            times: [u64; 4],
            counters: [i32; 12],
        }
        unsafe extern "C" {
            fn proc_pidinfo(
                pid: i32,
                flavor: i32,
                arg: u64,
                buffer: *mut std::ffi::c_void,
                size: i32,
            ) -> i32;
        }
        let mut info = Info {
            virtual_size: 0,
            resident_size: 0,
            times: [0; 4],
            counters: [0; 12],
        };
        let size = std::mem::size_of_val(&info) as i32;
        if unsafe {
            proc_pidinfo(
                std::process::id() as i32,
                4,
                0,
                &mut info as *mut _ as _,
                size,
            )
        } == size
        {
            return info.resident_size;
        }
    }
    0
}
#[test]
fn s1934_spool() -> Result<()> {
    let Some(root) = std::env::var_os("VZ_S1934_ROOT") else {
        return Ok(());
    };
    if std::env::var_os("VZ_S1934_PREPARE").is_none() {
        return Ok(());
    }
    let root = Path::new(&root);
    let start = Instant::now();
    let rows = harness::spool(
        Path::new(&std::env::var_os("VZ_S1934_SOURCE").context("source")?),
        &root.join("lookup.sqlite"),
    )?;
    harness::write_vectors(&root.join("lookup.sqlite"))?;
    let spool_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    tokio::runtime::Runtime::new()?.block_on(harness::prepare(
        &root.join("lookup.sqlite"),
        &root.join("base"),
    ))?;
    std::fs::write(
        root.join("preparation.json"),
        serde_json::to_vec_pretty(
            &json!({"rows":rows,"spool_ms":spool_ms,"lance_fts_ms":start.elapsed().as_secs_f64()*1000.,"rss_bytes":rss(),"batch_rows":1024,"batch_bytes":16777216}),
        )?,
    )?;
    Ok(())
}

#[test]
fn s1934_candidate() -> Result<()> {
    let Some(root) = std::env::var_os("VZ_S1934_ROOT") else {
        return Ok(());
    };
    let Ok(name) = std::env::var("VZ_S1934_CANDIDATE") else {
        return Ok(());
    };
    let root = Path::new(&root);
    let candidate = match name.as_str() {
        "exact" => Candidate::Exact,
        "streamed" => Candidate::StreamedExact,
        "flat" => Candidate::IvfFlat,
        "pq" => Candidate::IvfPq,
        "hnswpq" => Candidate::IvfHnswPq,
        "hnswsq" => Candidate::IvfHnswSq,
        _ => anyhow::bail!("candidate"),
    };
    tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()?.block_on(async {
        let engines=Arc::new(tokio::sync::Mutex::new(vz_core::engine::Engines::load_embedding(Path::new(&std::env::var_os("VZ_OFFICE_ASSETS").context("assets")?),Path::new(&std::env::var_os("VZ_OFFICE_PINS").context("pins")?),false)?));
        let baseline=rss(); let start=Instant::now();
        let snapshot=Snapshot::open(&root.join(&name),&root.join("lookup.sqlite"),candidate,54099).await?;
        snapshot.index().await?;
        let build_ms=start.elapsed().as_secs_f64()*1000.;
        let snapshot=Arc::new(snapshot);
        let queries:Vec<serde_json::Value>=serde_json::from_slice(&std::fs::read(root.join("queries.json"))?)?;
        let plan=snapshot.plan(&engines.lock().await.vector(queries[0]["query"].as_str().unwrap(),true)?).await?;
        let mut indices=Vec::new();
        for index in snapshot.table.list_indices().await? {indices.push(json!({"name":index.name,"statistics":format!("{:?}",snapshot.table.index_stats(&index.name).await?)}));}
        let mut samples=Vec::new();
        for mode in ["first","warm"] {
            let barrier=Arc::new(tokio::sync::Barrier::new(25)); let mut jobs=Vec::new();
            for (identity,q) in queries.iter().take(25).enumerate() {
                let snapshot=snapshot.clone(); let barrier=barrier.clone();let engines=engines.clone();let q=q.clone();
                let arrival=Instant::now();
                jobs.push(tokio::spawn(async move {
                    let scheduled_ms=arrival.elapsed().as_secs_f64()*1000.; barrier.wait().await;
                    let query=q["query"].as_str().unwrap();let start=Instant::now();
                    let engine=engines.lock().await;let guard_ms=start.elapsed().as_secs_f64()*1000.;let start=Instant::now();
                    let vector=engine.vector(query,true)?;let embed_ms=start.elapsed().as_secs_f64()*1000.;drop(engine);
                    let (hits,phases,top10)=snapshot.search(query,&vector,8).await?;
                    let latency_ms=arrival.elapsed().as_secs_f64()*1000.;
                    Ok::<_,anyhow::Error>(json!({"identity":identity,"mode":mode,"latency_ms":latency_ms,"guard_ms":guard_ms,"embed_ms":embed_ms,"scheduling_ms":scheduled_ms,"storage":phases,"correct_extraction_regression":hits.iter().any(|h|h.chunk.text.starts_with(query)),"vector_top10":top10,"documents":hits.iter().map(|h|&h.chunk.doc_id).collect::<Vec<_>>(),"query":query}))
                }));
            }
            for job in jobs {samples.push(job.await??);}
        }
        let after=rss();
        std::fs::write(root.join(format!("{name}.json")),serde_json::to_vec_pretty(&json!({"candidate":candidate,"index_build_ms":build_ms,"baseline_rss_bytes":baseline,"one_snapshot_rss_bytes":after,"incremental_rss_bytes":after.saturating_sub(baseline),"plan":plan,"indices":indices,"samples":samples,"training_sample_identity":"Lance random sample; exact row IDs not exposed: qualification gap","relevance":"extraction regression only; not human qualification","cold":"first open; no OS cache flush","refresh":"not measured in this candidate trial"}))?)?;
        Ok::<_,anyhow::Error>(())
    })
}
