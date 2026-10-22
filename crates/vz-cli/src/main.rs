//! Headless offline pipeline. Network is used only by the explicit upload command.
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{io::Write, path::PathBuf};
use vz_core::{engine::Engines, storage::State};
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
    let engines = Engines::load(&args.assets, &args.models, args.cpu)?;
    let runtime = tokio_runtime()?;
    runtime.block_on(async {
        match args.command {
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
            Operation::SelfTest | Operation::Upload { .. } => unreachable!(),
        }
        Ok(())
    })
}
fn tokio_runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Runtime::new()?)
}
