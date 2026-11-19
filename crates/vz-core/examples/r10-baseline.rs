//! R10 baseline driver. Compile this unchanged against ef34f139, then pass its
//! binary to VZ_R10_BASELINE_BIN for the ignored publication integration test.
use anyhow::Result;
use std::{path::Path, time::Instant};
use vz_core::{engine::Engines, storage::State};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let source = Path::new(&args[2]);
    let assets = Path::new(&args[3]);
    let pins = Path::new(&args[4]);
    let engines = Engines::load_embedding(assets, pins, true)?;
    let runtime = tokio::runtime::Runtime::new()?;
    let start = Instant::now();
    let mut state = State::open(root)?;
    let result = runtime.block_on(vz_core::index::index(
        &mut state,
        source,
        &engines,
        assets,
        |_| {},
    ))?;
    drop(state);
    println!(
        "{}",
        serde_json::json!({"seconds": start.elapsed().as_secs_f64(), "completed": result.completed, "skipped": result.skipped})
    );
    Ok(())
}
