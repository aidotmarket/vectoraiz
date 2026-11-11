#![allow(dead_code)]
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub struct Fixture {
    pub root: PathBuf,
    pub pack: PathBuf,
    pub source: PathBuf,
    pub staging: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "vz-export-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pack = root.join("pack");
        let source = root.join("source");
        let staging = root.join("staging");
        for p in [&pack, &source, &staging] {
            fs::create_dir(p).unwrap();
        }
        fs::create_dir(pack.join("examples")).unwrap();
        let payload = [
            ("documents.parquet", b"fixture parquet bytes".as_slice()),
            ("examples/query.py", b"print('fixture')".as_slice()),
        ];
        let mut files = serde_json::Map::new();
        for (name, bytes) in payload {
            fs::write(pack.join(name), bytes).unwrap();
            files.insert(name.into(), json!(vz_core::pipeline::sha256(bytes)));
        }
        fs::write(
            pack.join("manifest.json"),
            serde_json::to_vec(&json!({"format_version":1,"files":files})).unwrap(),
        )
        .unwrap();
        Self {
            root,
            pack,
            source,
            staging,
        }
    }
    pub fn assert_pack(&self, destination: &std::path::Path, report: &serde_json::Value) {
        assert_eq!(report["files"].as_array().unwrap().len(), 3);
        for file in report["files"].as_array().unwrap() {
            let name = file["path"].as_str().unwrap();
            let bytes = fs::read(destination.join(name)).unwrap();
            assert_eq!(bytes, fs::read(self.pack.join(name)).unwrap());
            assert_eq!(file["size"], bytes.len() as u64);
            assert_eq!(file["sha256"], vz_core::pipeline::sha256(&bytes));
        }
    }
    pub fn assert_source_empty(&self) {
        assert!(walkdir::WalkDir::new(&self.source)
            .into_iter()
            .all(|e| !e.unwrap().file_type().is_file()));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
