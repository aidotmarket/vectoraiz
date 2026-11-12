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

#[test]
fn s1934_refresh() -> Result<()> {
    use rusqlite::{params, Connection};
    use vz_core::pipeline::Chunk;
    let Some(root) = std::env::var_os("VZ_S1934_REFRESH") else {
        return Ok(());
    };
    let root = Path::new(&root);
    tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()?.block_on(async {
        let engine = vz_core::engine::Engines::load_embedding(
            Path::new(&std::env::var_os("VZ_OFFICE_ASSETS").context("assets")?),
            Path::new(&std::env::var_os("VZ_OFFICE_PINS").context("pins")?),false)?;
        let baseline = rss();
        let old = Arc::new(Snapshot::open(&root.join("exact"), &root.join("lookup.sqlite"),Candidate::Exact,54099).await?);
        let start = Instant::now();
        let noop = old.clone();
        let noop_ms = start.elapsed().as_secs_f64()*1000.;
        assert!(Arc::ptr_eq(&old,&noop)); drop(noop);
        let start = Instant::now();
        for (source,destination) in [(root.join("exact"),root.join("refresh")),(root.join("lookup.sqlite"),root.join("refresh.sqlite"))] {
            anyhow::ensure!(std::process::Command::new("rtk").args(["proxy","/bin/cp","-cR"]).arg(source).arg(destination).status()?.success(),"private CoW copy");
        }
        let clone_ms = start.elapsed().as_secs_f64()*1000.;
        let mut db = Connection::open(root.join("refresh.sqlite"))?;
        let mut changed:Vec<Chunk> = db.prepare("SELECT payload FROM chunks ORDER BY id LIMIT 360")?.query_map([],|r|r.get::<_,Vec<u8>>(0))?.map(|r|Ok(serde_json::from_slice(&r?)?)).collect::<Result<_>>()?;
        let removed = changed.iter().map(|c|c.chunk_id.clone()).collect::<Vec<_>>();
        let start = Instant::now();
        let mut additions = Vec::new();
        for (i,chunk) in changed.iter_mut().take(180).enumerate() {
            chunk.text.push_str(" Revised office reference."); chunk.embedding=engine.vector(&chunk.text,false)?;
            let mut addition=chunk.clone();addition.chunk_id=format!("refresh-add-{i:04}");addition.doc_id=format!("refresh-add-doc-{i:04}");additions.push(addition);
        }
        changed.truncate(180);changed.extend(additions);
        let embedding_ms=start.elapsed().as_secs_f64()*1000.;
        let start=Instant::now();
        let next=Snapshot::open(&root.join("refresh"),&root.join("refresh.sqlite"),Candidate::Exact,54099).await?;
        let tx=db.transaction()?;
        for id in &removed {tx.execute("DELETE FROM chunks WHERE id=?1",[id])?;}
        for chunk in &changed {tx.execute("INSERT INTO chunks(id,payload) VALUES(?1,?2)",params![chunk.chunk_id,serde_json::to_vec(chunk)?])?;}
        tx.commit()?;drop(db);
        let filter=format!("chunk_id IN ({})",removed.iter().map(|id|format!("'{}'",id.replace('\'',"''"))).collect::<Vec<_>>().join(","));
        next.table.delete(&filter).await?;
        let batch=vz_core::storage::chunks_batch(&changed)?;
        let reader=arrow_array::RecordBatchIterator::new(vec![Ok(batch.clone())],batch.schema());
        next.table.add(Box::new(reader)).execute().await?;
        // N2: Exact has no vector index to update; optimize the selected
        // path's FTS index and refresh the bounded contiguous representation.
        next.table.optimize(lancedb::table::OptimizeAction::Index(Default::default())).await?;
        harness::write_vectors(&root.join("refresh.sqlite"))?;
        let update_ms=start.elapsed().as_secs_f64()*1000.;
        let two_snapshot_rss=rss();
        let vector=engine.vector("office reference",true)?;
        let _=old.search("office reference",&vector,8).await?;
        let _=next.search("office reference",&vector,8).await?;
        let weak=Arc::downgrade(&old);let reader=old.clone();drop(old);
        assert!(weak.upgrade().is_some());let start=Instant::now();drop(reader);assert!(weak.upgrade().is_none());
        std::fs::write(root.join("refresh.json"),serde_json::to_vec_pretty(&json!({"candidate":"Exact","no_op_ms":noop_ms,"no_op_reuses_same_handle":true,"clone_ms":clone_ms,"changed_chunks":540,"updates":180,"adds":180,"deletes":180,"embedding_ms":embedding_ms,"index_update_and_vector_file_ms":update_ms,"post_embedding_preparation_ms":clone_ms+update_ms,"baseline_rss_bytes":baseline,"two_snapshot_rss_bytes":two_snapshot_rss,"aggregate_incremental_rss_bytes":two_snapshot_rss.saturating_sub(baseline),"retirement_ms":start.elapsed().as_secs_f64()*1000.,"vector_index_update":"not applicable to exact path (N2)","limits":"synthetic chunk mutations; source hash/extraction accounting and concurrent 25-way Refresh search not qualified; no product promotion"}))?)?;
        Ok::<_,anyhow::Error>(())
    })
}
