//! Document identities and deterministic structure-aware chunk boundaries.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn file_sha256(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub page: u32,
    pub text: String,
    pub needs_ocr: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunk {
    pub chunk_id: String,
    pub doc_id: String,
    pub text: String,
    pub page: u32,
    pub heading_path: Vec<String>,
    pub token_count: usize,
    pub embedding: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub doc_id: String,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub pages: u32,
    pub kind: String,
    pub chunks: Vec<Chunk>,
}

/// Conservative token estimate: one token per four UTF-8 bytes, whitespace aligned.
/// Headings and pages always start a new chunk. This estimate is explicitly in the recipe.
pub fn chunk(doc_id: &str, pages: &[Page]) -> Vec<Chunk> {
    let mut result = Vec::new();
    let mut headings: Vec<String> = Vec::new();
    for page in pages {
        let mut words: Vec<&str> = Vec::new();
        let emit = |words: &[&str], headings: &[String], result: &mut Vec<Chunk>| {
            if words.is_empty() {
                return;
            }
            let text = words.join(" ");
            result.push(Chunk {
                chunk_id: format!("{doc_id}-{}", result.len()),
                doc_id: doc_id.into(),
                token_count: text.len().div_ceil(4),
                text,
                page: page.page,
                heading_path: headings.to_vec(),
                embedding: Vec::new(),
            });
        };
        for line in page.text.lines() {
            let level = line.chars().take_while(|c| *c == '#').count();
            if level > 0 && line.as_bytes().get(level) == Some(&b' ') {
                emit(&words, &headings, &mut result);
                words.clear();
                headings.truncate(level - 1);
                headings.push(line[level..].trim().into());
            }
            for word in line.split_whitespace() {
                // An oversized word is split at UTF-8 boundaries, so hostile input cannot
                // create unbounded model input or an invalid string slice.
                let mut start = 0;
                while start < word.len() {
                    let mut end = (start + 1800).min(word.len());
                    while !word.is_char_boundary(end) {
                        end -= 1;
                    }
                    let piece = &word[start..end];
                    if words.iter().map(|w| w.len() + 1).sum::<usize>() + piece.len() > 2000 {
                        emit(&words, &headings, &mut result);
                        let mut overlap = 0;
                        let keep = words
                            .iter()
                            .rev()
                            .take_while(|w| {
                                overlap += w.len() + 1;
                                overlap <= 160
                            })
                            .count();
                        words.drain(..words.len() - keep);
                    }
                    words.push(piece);
                    start = end;
                }
            }
        }
        emit(&words, &headings, &mut result);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_overlap_and_provenance() {
        let text = format!("# Introduction\n{}\n## Detail\nend", "word ".repeat(1600));
        let chunks = chunk(
            "doc",
            &[Page {
                page: 3,
                text,
                needs_ocr: false,
            }],
        );
        assert!(chunks.len() > 3);
        assert!(chunks
            .iter()
            .all(|c| c.doc_id == "doc" && c.page == 3 && c.token_count <= 500));
        assert_eq!(
            chunks.last().unwrap().heading_path,
            ["Introduction", "Detail"]
        );
        assert!(chunks[0].text.ends_with("word"));
        assert!(chunks[1].text.starts_with("word"));
    }
    #[test]
    fn unicode_and_page_boundaries() {
        let pages = [
            Page {
                page: 1,
                text: "界".repeat(2000),
                needs_ocr: false,
            },
            Page {
                page: 2,
                text: "next".into(),
                needs_ocr: false,
            },
        ];
        let chunks = chunk("d", &pages);
        assert!(chunks.iter().all(|c| c.text.len() <= 2000));
        assert_eq!(chunks.last().unwrap().page, 2);
    }
}
