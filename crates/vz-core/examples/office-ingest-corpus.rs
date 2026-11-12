//! Ordinary-ingest corpus probe. Reports aggregate pages and per-file results.
use std::{collections::BTreeMap, path::Path};
fn visit(root: &Path, files: &mut Vec<std::path::PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            visit(&path, files)?;
        } else if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| ["docx", "xlsx", "pptx"].contains(&s))
        {
            files.push(path);
        }
    }
    Ok(())
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mut files = Vec::new();
    visit(Path::new(&args[1]), &mut files)?;
    files.sort();
    let mut details = Vec::new();
    let mut totals = BTreeMap::<String, (usize, usize, usize)>::new();
    for path in files {
        let ext = path.extension().unwrap().to_str().unwrap().to_owned();
        let started = std::time::Instant::now();
        let result = vz_core::parsers::parse(&vz_core::worker::Request {
            path: path.clone(),
            pdfium: None,
            office: None,
        });
        let (pages, error) = match result {
            Ok(pages) => (pages.len(), None),
            Err(e) => (0, Some(e.to_string())),
        };
        let total = totals.entry(ext.clone()).or_default();
        total.0 += 1;
        total.1 += usize::from(error.is_none());
        total.2 += pages;
        details.push(serde_json::json!({"path": path, "format":ext,"pages":pages,"error":error,"seconds":started.elapsed().as_secs_f64()}));
    }
    std::fs::write(&args[2], serde_json::to_vec_pretty(&details)?)?;
    println!("{}", serde_json::to_string_pretty(&totals)?);
    Ok(())
}
