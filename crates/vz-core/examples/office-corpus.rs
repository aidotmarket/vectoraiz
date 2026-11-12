//! Local aggregate-only corpus probe; file paths/content never printed.
use std::{collections::BTreeMap, path::Path};
fn main() -> anyhow::Result<()> {
    let root = std::env::args()
        .nth(1)
        .expect("usage: office-corpus DIRECTORY");
    let mut results = BTreeMap::<String, BTreeMap<String, usize>>::new();
    let mut files = Vec::new();
    fn visit(path: &Path, out: &mut Vec<std::path::PathBuf>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(&path, out)?;
            } else if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| ["docx", "xlsx", "pptx"].contains(&s))
            {
                out.push(path);
            }
        }
        Ok(())
    }
    visit(Path::new(&root), &mut files)?;
    files.sort();
    let mut cache = BTreeMap::new();
    for path in files {
        let ext = path.extension().unwrap().to_str().unwrap();
        let bytes = std::fs::read(&path)?;
        let name = format!("document.{ext}");
        let key = (vz_core::pipeline::sha256(&bytes), ext.to_owned());
        let reason = cache
            .entry(key)
            .or_insert_with(|| {
                match vz_core::office::rebuild(&name, &bytes, &Default::default())
                    .and_then(|(bytes, _)| vz_core::office::inspect(&name, &bytes))
                {
                    Ok(_) => "PASS".to_owned(),
                    Err(e) => e.to_string(),
                }
            })
            .clone();
        *results
            .entry(ext.into())
            .or_default()
            .entry(reason)
            .or_default() += 1;
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
