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
    let result = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?
        .block_on(harness::prepare_streamed(
            Path::new(&std::env::var_os("VZ_S1934_SOURCE").context("source")?),
            &root.join("lookup.sqlite"),
            &root.join("base"),
        ))?;
    std::fs::write(
        root.join("preparation.json"),
        serde_json::to_vec_pretty(&result)?,
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
        std::fs::write(root.join(format!("{name}.json")),serde_json::to_vec_pretty(&json!({"candidate":candidate,"access_recipe":{"embedding":engines.lock().await.recipe,"vector_path":candidate,"metric":"L2","partitions":snapshot.partitions,"probes":snapshot.partitions.min(32),"candidates":40,"rrf_k":60,"weights":[1,1],"PQ_subvectors":48,"PQ_bits":8,"sample_rate":128,"iterations":50,"HNSW_edges":20,"HNSW_ef_construction":300,"HNSW_ef":256,"refine_factor":4,"path_change_requires_requalification":true},"source_hashes":{"core":vz_core::pipeline::sha256(include_bytes!("../src/storage/office_s1934.rs")),"test":vz_core::pipeline::sha256(include_bytes!("office_s1934.rs"))},"index_build_ms":build_ms,"baseline_rss_bytes":baseline,"one_snapshot_rss_bytes":after,"incremental_rss_bytes":after.saturating_sub(baseline),"plan":plan,"indices":indices,"samples":samples,"training_sample_identity":"Lance random sample; exact row IDs not exposed: qualification gap","relevance":"extraction regression only; not human qualification","cold":"first open; no OS cache flush","refresh":"not measured in this candidate trial"}))?)?;
        Ok::<_,anyhow::Error>(())
    })
}

#[test]
fn s1934_refresh() -> Result<()> {
    use rusqlite::Connection;
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
        let mut changed:Vec<Chunk> = db.prepare("SELECT id,doc_id,text,page,heading,tokens FROM chunks ORDER BY id LIMIT 360")?.query_map([],harness::lookup_chunk)?.collect::<rusqlite::Result<_>>()?;
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
        {let mut insert=tx.prepare(harness::LOOKUP_INSERT)?;for chunk in &changed {harness::insert_lookup(&mut insert,chunk)?;}}
        tx.commit()?;drop(db);
        let filter=format!("chunk_id IN ({})",removed.iter().map(|id|format!("'{}'",id.replace('\'',"''"))).collect::<Vec<_>>().join(","));
        next.table.delete(&filter).await?;
        let batch=vz_core::storage::chunks_batch(&changed)?;
        let reader=arrow_array::RecordBatchIterator::new(vec![Ok(batch.clone())],batch.schema());
        next.table.add(Box::new(reader)).execute().await?;
        // N2: Exact has no vector index to update; optimize the selected
        // path's FTS index and refresh the bounded contiguous representation.
        next.table.optimize(lancedb::table::OptimizeAction::Index(Default::default())).await?;
        // The selected Lance exact scan reads the new table version directly.
        // There is no vector index or contiguous-file representation to update.
        let update_ms=start.elapsed().as_secs_f64()*1000.;
        let two_snapshot_rss=rss();
        let vector=engine.vector("office reference",true)?;
        let _=old.search("office reference",&vector,8).await?;
        let _=next.search("office reference",&vector,8).await?;
        let weak=Arc::downgrade(&old);let reader=old.clone();drop(old);
        assert!(weak.upgrade().is_some());let start=Instant::now();drop(reader);assert!(weak.upgrade().is_none());
        std::fs::write(root.join("refresh.json"),serde_json::to_vec_pretty(&json!({"candidate":"Exact","no_op_ms":noop_ms,"no_op_reuses_same_handle":true,"clone_ms":clone_ms,"changed_chunks":540,"updates":180,"adds":180,"deletes":180,"embedding_ms":embedding_ms,"fts_update_ms":update_ms,"fts_update_operation":"Lance OptimizeAction::Index: incremental tail indexing plus merge; not create_index full rebuild","post_embedding_preparation_ms":clone_ms+update_ms,"baseline_rss_bytes":baseline,"two_snapshot_rss_bytes":two_snapshot_rss,"aggregate_incremental_rss_bytes":two_snapshot_rss.saturating_sub(baseline),"retirement_ms":start.elapsed().as_secs_f64()*1000.,"vector_index_update":"not applicable to exact path (N2)","limits":"synthetic chunk mutations; source hash/extraction accounting and concurrent 25-way Refresh search not qualified; no product promotion"}))?)?;
        Ok::<_,anyhow::Error>(())
    })
}

#[test]
fn s1934_relevance() -> Result<()> {
    use std::io::{BufRead, BufReader};
    use vz_core::pipeline::{chunk, Page};
    let Some(root) = std::env::var_os("VZ_S1934_RELEVANCE") else {
        return Ok(());
    };
    let root = Path::new(&root);
    tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()?.block_on(async {
        let engine = vz_core::engine::Engines::load_embedding(
            Path::new(&std::env::var_os("VZ_OFFICE_ASSETS").context("assets")?),
            Path::new(&std::env::var_os("VZ_OFFICE_PINS").context("pins")?),false)?;
        let snapshot = Snapshot::open(&root.join("lance"), &root.join("combined.sqlite"),Candidate::IvfFlat,54099).await?;
        let mut lookup = rusqlite::Connection::open(root.join("combined.sqlite"))?;
        let mut pending = Vec::new(); let mut count=0;
        let mut scifact_mapping = Vec::new();
        for line in BufReader::new(std::fs::File::open(root.join("scifact/corpus.jsonl"))?).lines() {
            let document:serde_json::Value=serde_json::from_str(&line?)?;
            let id=format!("scifact-{}",document["_id"].as_str().context("id")?);
            let pages=[Page {page:1,text:format!("{}\n{}",document["title"].as_str().unwrap_or(""),document["text"].as_str().context("abstract")?),needs_ocr:false}];
            let mut chunks=chunk(&id,&pages);
            for batch in chunks.chunks_mut(32) {for chunk in batch {chunk.embedding=engine.vector(&chunk.text,false)?;}}
            for row in &chunks {scifact_mapping.push(json!({"chunk_id":row.chunk_id,"doc_id":row.doc_id,"page":row.page}));}
            pending.extend(chunks);count+=1;
            if pending.len()>=512 {
                append_relevance(&snapshot,&mut lookup,&pending).await?;pending.clear();
                std::fs::write(root.join("embedding-progress.json"),json!({"documents":count}).to_string())?;
            }
        }
        if !pending.is_empty() {append_relevance(&snapshot,&mut lookup,&pending).await?;}
        std::fs::write(root.join("scifact-chunk-mapping.json"),serde_json::to_vec_pretty(&scifact_mapping)?)?;
        let office: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(root.join("office-extracted.json"))?)?;
        let mut office_chunks = 0;
        let mut mapping = Vec::new();
        for document in &office {
            let pages: Vec<Page> = serde_json::from_value(document["pages"].clone())?;
            let mut chunks = chunk(document["id"].as_str().context("office id")?, &pages);
            for batch in chunks.chunks_mut(32) {
                for chunk in batch {
                    chunk.embedding = engine.vector(&chunk.text, false)?;
                    mapping.push(json!({"chunk_id":chunk.chunk_id,"doc_id":chunk.doc_id,"page":chunk.page}));
                }
            }
            office_chunks += chunks.len();
            append_relevance(&snapshot, &mut lookup, &chunks).await?;
        }
        std::fs::write(root.join("office-chunk-mapping.json"), serde_json::to_vec_pretty(&mapping)?)?;
        let optimize=Instant::now();
        snapshot.table.optimize(lancedb::table::OptimizeAction::Index(Default::default())).await?;
        let optimize_ms=optimize.elapsed().as_secs_f64()*1000.;
        // Build ANN only after every SciFact/office row is appended. Otherwise
        // a labelled IVF trial could silently measure an unindexed exact tail.
        let index_start = Instant::now();
        snapshot.index().await?;
        let index_ms = index_start.elapsed().as_secs_f64()*1000.;
        drop(snapshot);drop(lookup);
        let exact=Snapshot::open(&root.join("lance"),&root.join("combined.sqlite"),Candidate::Exact,54099).await?;
        let candidate=Snapshot::open(&root.join("lance"),&root.join("combined.sqlite"),Candidate::IvfFlat,54099).await?;
        let ids=std::fs::read_to_string(root.join("scifact/qrels/test.tsv"))?.lines().skip(1).filter_map(|l|l.split('\t').next().map(str::to_owned)).collect::<std::collections::HashSet<_>>();
        let mut runs=serde_json::Map::new();runs.insert("exact".into(),json!([]));runs.insert("ivf_flat".into(),json!([]));runs.insert("selected_exact".into(),json!([]));
        for line in BufReader::new(std::fs::File::open(root.join("scifact/queries.jsonl"))?).lines() {
            let query:serde_json::Value=serde_json::from_str(&line?)?;let id=query["_id"].as_str().context("query id")?;
            if !ids.contains(id) {continue;}
            let text=query["text"].as_str().context("query text")?;let vector=engine.vector(text,true)?;
            let reference=exact.vector(&vector).await?.into_iter().take(10).map(|(id,_)|id).collect::<std::collections::HashSet<_>>();
            for (name,snapshot) in [("exact",&exact),("selected_exact",&exact),("ivf_flat",&candidate)] {
                let start=Instant::now();let (hits,phases,top)=snapshot.search(text,&vector,80).await?;
                let recall=top.iter().filter(|id|reference.contains(*id)).count() as f64/10.;
                runs[name].as_array_mut().unwrap().push(json!({"id":id,"documents":({let mut seen=std::collections::HashSet::new();hits.iter().filter(|h|seen.insert(h.chunk.doc_id.clone())).take(10).map(|h|h.chunk.doc_id.clone()).collect::<Vec<_>>()}),"passages":hits.iter().map(|h|&h.chunk.text).collect::<Vec<_>>(),"latency_ms":start.elapsed().as_secs_f64()*1000.,"phases":phases,"vector_recall_at10":recall}));
            }
            std::fs::write(root.join("scifact-run.json"),serde_json::to_vec_pretty(&json!({"runs":runs,"scope":{"load_documents":1000,"load_chunks":54099,"scifact_documents":count,"scifact_chunks":scifact_mapping.len(),"selected_path":"Exact","baseline":"exact vector + identical FTS/RRF","document_ranking":"deduplicate full fused 80-candidate chunk pool before top10","office_documents":office.len(),"office_chunks":office_chunks,"office":"extracted drafts indexed; unjudged queries excluded from human scoring","parameters":"E3 P256, probes32, refine4, L2, 40 vector + 40 FTS, RRF60","optimize_ms":optimize_ms,"index_ms":index_ms,"recipe":engine.recipe}}))?)?;
        }
        let drafts: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(root.join("office-drafts.json"))?)?;
        let mut office_runs = Vec::new();
        for query in drafts {
            let text = query["query"].as_str().context("draft query")?;
            let vector = engine.vector(text, true)?;
            for (name, snapshot) in [("exact", &exact), ("selected_exact", &exact), ("ivf_flat", &candidate)] {
                let start = Instant::now();
                let (hits, phases, _) = snapshot.search(text, &vector, 10).await?;
                office_runs.push(json!({"path":name,"id":query["id"],"format":query["format"],"language":query["language"],"documents":({let mut seen=std::collections::HashSet::new();hits.iter().filter(|h|seen.insert(h.chunk.doc_id.clone())).take(10).map(|h|h.chunk.doc_id.clone()).collect::<Vec<_>>()}),"passages":hits.iter().map(|h|&h.chunk.text).collect::<Vec<_>>(),"latency_ms":start.elapsed().as_secs_f64()*1000.,"phases":phases,"judgment_status":"UNJUDGED; author-intent diagnostics only"}));
            }
        }
        std::fs::write(root.join("office-run.json"), serde_json::to_vec_pretty(&office_runs)?)?;
        std::fs::write(root.join("relevance-plan.json"), serde_json::to_vec_pretty(&json!({"exact":exact.plan(&engine.vector("scientific claim",true)?).await?,"ivf_flat":candidate.plan(&engine.vector("scientific claim",true)?).await?,"indices":format!("{:?}",candidate.table.list_indices().await?)}))?)?;
        Ok::<_,anyhow::Error>(())
    })
}

async fn append_relevance(
    snapshot: &Snapshot,
    lookup: &mut rusqlite::Connection,
    chunks: &[vz_core::pipeline::Chunk],
) -> Result<()> {
    for chunks in chunks.chunks(512) {
        let batch = vz_core::storage::chunks_batch(chunks)?;
        anyhow::ensure!(
            batch.get_array_memory_size() <= 16 * 1024 * 1024,
            "relevance batch budget"
        );
        let tx = lookup.transaction()?;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO chunks(id,payload) VALUES(?1,?2)",
                rusqlite::params![chunk.chunk_id, serde_json::to_vec(chunk)?],
            )?;
        }
        let reader = arrow_array::RecordBatchIterator::new(vec![Ok(batch.clone())], batch.schema());
        snapshot.table.add(Box::new(reader)).execute().await?;
        tx.commit()?;
    }
    Ok(())
}
