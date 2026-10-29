//! Headless pipeline. Network is used by explicit upload and SFTP export commands.
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{io::Write, path::PathBuf};
use vz_core::{
    engine::{ChatOnly, Engines},
    storage::State,
};
#[derive(Parser)]
#[command(name = "vectoraiz-cli")]
struct Args {
    #[arg(long, default_value = ".vz-index")]
    state: PathBuf,
    #[arg(long, default_value = ".assets")]
    assets: PathBuf,
    #[arg(long, default_value = "spike/fixtures/models.json")]
    models: PathBuf,
    #[arg(long)]
    cpu: bool,
    #[command(subcommand)]
    command: Operation,
}
#[derive(Subcommand)]
enum Operation {
    BakeoffSearch {
        #[arg(long)]
        questions: PathBuf,
        #[arg(long, default_value_t = 10)]
        top_k: usize,
    },
    BakeoffChat {
        #[arg(long)]
        items: PathBuf,
        #[arg(long, value_parser = ["answer", "cite"])]
        prompt: String,
    },
    Export {
        pack: PathBuf,
        #[arg(long)]
        pack_id: String,
        #[arg(
            long,
            required_unless_present = "sftp",
            conflicts_with = "sftp",
            requires = "staging"
        )]
        folder: Option<PathBuf>,
        #[arg(long, requires = "folder")]
        staging: Option<PathBuf>,
        #[arg(
            long,
            required_unless_present = "folder",
            conflicts_with = "folder",
            requires = "sftp_staging"
        )]
        sftp: Option<String>,
        #[arg(long, requires = "sftp")]
        sftp_staging: Option<String>,
        #[arg(long, requires = "sftp")]
        identity: Option<PathBuf>,
        #[arg(long, requires = "sftp")]
        known_hosts: Option<PathBuf>,
    },
    SelfTest,
    Embed {
        text: String,
    },
    Index {
        folder: PathBuf,
    },
    Search {
        query: String,
        #[arg(long, default_value_t = 5)]
        top_k: usize,
    },
    Ask {
        question: String,
    },
    Pack {
        output: PathBuf,
        #[arg(long, default_value = "What is the orbital archive retention policy?")]
        query: String,
    },
    Upload {
        pack: PathBuf,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        bucket: String,
        #[arg(long, default_value = "us-east-1")]
        region: String,
        #[arg(long, default_value = "vz/phase0-pack")]
        prefix: String,
    },
}
fn main() -> Result<()> {
    let args = Args::parse();
    if let Operation::Export {
        pack,
        pack_id,
        folder,
        staging,
        sftp,
        sftp_staging,
        identity,
        known_hosts,
    } = &args.command
    {
        let result = if let Some(source) = folder {
            vz_pack::export::folder(
                pack,
                pack_id,
                source,
                staging.as_deref().expect("clap requires staging"),
            )?
        } else {
            tokio_runtime()?.block_on(vz_pack::export::sftp(
                pack,
                pack_id,
                &vz_pack::export::SftpOptions {
                    target: sftp.clone().expect("clap requires mode"),
                    staging: sftp_staging.clone().expect("clap requires SFTP staging"),
                    identity: identity.clone(),
                    known_hosts: known_hosts.clone(),
                },
            ))?
        };
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    if matches!(args.command, Operation::SelfTest) {
        println!("{}", serde_json::to_string(&vz_core::self_test())?);
        return Ok(());
    }
    if let Operation::Upload {
        pack,
        endpoint,
        bucket,
        region,
        prefix,
    } = &args.command
    {
        return vz_pack::upload(pack, endpoint, bucket, region, prefix);
    }
    if let Operation::BakeoffChat { items, prompt } = &args.command {
        let chat = ChatOnly::load(&args.assets, &args.models, args.cpu)?;
        for item in read_questions(items)? {
            let sources = item["sources"].as_array().context("sources array")?;
            let context = sources
                .iter()
                .map(|s| {
                    Ok(format!(
                        "[{}] {}",
                        s["n"].as_u64().context("source number")?,
                        s["text"].as_str().context("source text")?
                    ))
                })
                .collect::<Result<Vec<_>>>()?
                .join("\n\n");
            let result = chat.measure(
                item["question"].as_str().context("question")?,
                &context,
                prompt == "cite",
            )?;
            let mut value = serde_json::to_value(result)?;
            value["id"] = item.get("id").context("id")?.clone();
            println!("{}", value);
        }
        return Ok(());
    }
    let engines = if matches!(args.command, Operation::BakeoffSearch { .. }) {
        Engines::load_embedding(&args.assets, &args.models, args.cpu)?
    } else {
        Engines::load(&args.assets, &args.models, args.cpu)?
    };
    let runtime = tokio_runtime()?;
    runtime.block_on(async {
        match args.command {
            Operation::BakeoffSearch { questions, top_k } => {
                let (state, _) = State::open_index(&args.state, &engines.recipe)?;
                let paths = state.documents()?.into_iter().map(|d| (d.doc_id, d.path)).collect::<std::collections::HashMap<_, _>>();
                for item in read_questions(&questions)? {
                    let question = item["question"].as_str().context("question")?;
                    let started = std::time::Instant::now();
                    let hits = state.search(question, &engines.vector(question, true)?, top_k.clamp(1, 100)).await?;
                    let query_ms = started.elapsed().as_secs_f64() * 1000.0;
                    let hits = hits.iter().enumerate().map(|(i, h)| Ok(serde_json::json!({"rank": i + 1, "doc_path": paths.get(&h.chunk.doc_id).context("hit document")?, "page": h.chunk.page, "chunk_id": h.chunk.chunk_id, "score": h.score}))).collect::<Result<Vec<_>>>()?;
                    println!("{}", serde_json::json!({"id": item.get("id").context("id")?, "hits": hits, "query_ms": query_ms}));
                }
            }
            Operation::Embed { text } => { println!("{}",serde_json::to_string(&serde_json::json!({"embedding":engines.vector(&text,true)?,"recipe":engines.recipe}))?); }
            Operation::Index { folder } => {
                let mut state = State::open(&args.state)?;
                let result =
                    vz_core::index::index(&mut state, &folder, &engines, &args.assets, |p| {
                        eprintln!("{}", serde_json::to_string(p).unwrap())
                    })
                    .await?;
                println!("{}", serde_json::to_string(&result)?);
            }
            Operation::Search { query, top_k } => {
                let (state, _) = State::open_index(&args.state, &engines.recipe)?;
                let hits = state
                    .search(&query, &engines.vector(&query, true)?, top_k.clamp(1, 100))
                    .await?;
                println!("{}", serde_json::to_string(&hits)?);
            }
            Operation::Ask { question } => {
                let (state, _) = State::open_index(&args.state, &engines.recipe)?;
                let hits = state
                    .search(&question, &engines.vector(&question, true)?, 3)
                    .await?;
                let answer = vz_core::index::ask(&engines, &question, &hits, |token| {
                    eprint!("{token}");
                    let _ = std::io::stderr().flush();
                })?;
                eprintln!();
                println!("{}", serde_json::to_string(&answer)?);
            }
            Operation::Pack { output, query } => {
                let manifest = vz_pack::write(&args.state, &engines, &output, &query)?;
                println!("{}", serde_json::to_string(&manifest)?);
            }
            Operation::SelfTest
            | Operation::Upload { .. }
            | Operation::BakeoffChat { .. }
            | Operation::Export { .. } => unreachable!(),
        }
        Ok(())
    })
}
fn tokio_runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Runtime::new()?)
}

fn read_questions(path: &std::path::Path) -> Result<Vec<serde_json::Value>> {
    let text = std::fs::read_to_string(path)?;
    if text.trim_start().starts_with('[') {
        return Ok(serde_json::from_str(&text)?);
    }
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}
