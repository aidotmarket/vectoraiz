//! Private, opt-in timing probe. Run from the repository root with VZ_ENGINE_TIMING=1.
//! Build with `cargo build -p vz-core --example engine-timing`, copy the binary
//! beside vz-parser-worker in target/debug, then pass STATE SOURCE [--cpu] [--cached].
//! Uses isolated caller-provided state/source directories; emits no answer text.
use anyhow::Result;
use std::{path::Path, time::Instant};
use vz_core::{engine::Engines, storage::State};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let cpu = args.iter().any(|arg| arg == "--cpu");
    let root = Path::new(&args[1]);
    let source = Path::new(&args[2]);
    let assets = Path::new(".assets");
    let pins = Path::new("spike/fixtures/models.json");
    let rt = tokio::runtime::Runtime::new()?;
    let query = "What is the orbital archive retention policy?";
    let cached = args.iter().any(|arg| arg == "--cached");
    let mut cache = None;
    for command in ["index", "search1", "search2", "ask1", "ask2"] {
        eprintln!("TIMING_COMMAND {command}");
        let start = Instant::now();
        if !cached {
            cache = None;
        }
        if cache.is_none() {
            cache = Some(if cached {
                Engines::load_embedding(assets, pins, cpu)?
            } else {
                Engines::load(assets, pins, cpu)?
            });
        }
        let engines = cache.as_ref().unwrap();
        if cached && !command.starts_with("ask") {
            anyhow::ensure!(!engines.chat_loaded(), "Index/Search loaded chat");
        }
        if command.starts_with("ask") {
            engines.load_chat()?;
        }
        if command == "index" {
            let mut state = State::open(root)?;
            rt.block_on(vz_core::index::index(
                &mut state,
                source,
                engines,
                assets,
                |_| {},
            ))?;
        } else {
            let (state, _) = State::open_index(root, &engines.recipe)?;
            let hits = rt.block_on(state.search(
                query,
                &engines.vector(query, true)?,
                if command.starts_with("ask") { 3 } else { 5 },
            ))?;
            if command.starts_with("ask") {
                let answer = vz_core::index::ask(engines, query, &hits, |_| {})?;
                anyhow::ensure!(
                    answer.text.to_lowercase().contains("thirty days"),
                    "unexpected answer"
                );
            }
        }
        println!("{command} {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}
