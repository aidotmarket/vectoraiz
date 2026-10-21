//! Minimal portable vector packs and explicitly requested S3-compatible upload.
use anyhow::{bail, Context, Result};
use parquet::arrow::ArrowWriter;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs::File, io::Write, path::Path};
use vz_core::{
    engine::{Engines, Recipe},
    pipeline::sha256,
    storage::{chunks_batch, documents_batch, State},
};

pub fn write(state: &State, engines: &Engines, output: &Path, query: &str) -> Result<Value> {
    if output.exists() {
        bail!("pack output already exists; choose an empty destination");
    }
    let docs = state.documents()?;
    if docs.is_empty() {
        bail!("index is empty");
    }
    // Stage beside the destination and expose only after all files and hashes exist.
    let stage = output.with_extension("partial");
    if stage.exists() {
        bail!("interrupted pack exists; remove the .partial directory before retrying");
    }
    std::fs::create_dir_all(stage.join("examples"))?;
    let result = (|| {
        let chunks = docs
            .iter()
            .flat_map(|d| d.chunks.clone())
            .collect::<Vec<_>>();
        for (name, batch) in [
            ("documents.parquet", documents_batch(&docs)?),
            ("chunks.parquet", chunks_batch(&chunks)?),
        ] {
            let mut writer =
                ArrowWriter::try_new(File::create(stage.join(name))?, batch.schema(), None)?;
            writer.write(&batch)?;
            writer.close()?;
        }
        let mut zip = zip::ZipWriter::new(File::create(stage.join("originals-0001.zip"))?);
        let mut original_bytes = 0_u64;
        for (i, doc) in docs.iter().enumerate() {
            let bytes = std::fs::read(&doc.path).context("original source unavailable")?;
            if sha256(&bytes) != doc.sha256 {
                bail!("source changed since indexing; re-index before packing");
            }
            original_bytes += bytes.len() as u64;
            if original_bytes > 2 * 1024 * 1024 * 1024 {
                bail!("Phase 0 pack originals exceed 2 GB; split the collection");
            }
            zip.start_file(
                format!("document-{:04}.{}", i + 1, doc.kind),
                zip::write::SimpleFileOptions::default(),
            )?;
            zip.write_all(&bytes)?;
        }
        zip.finish()?.sync_all()?;
        std::fs::write(stage.join("README.md"), "# vectorAIz vector pack\n\nLoad documents.parquet and chunks.parquet with Apache Arrow.\nThe manifest records the exact embedding and chunker recipe.\nOriginal file names are neutral. Documents retain their source licences.\nSee examples/query.py for a verified offline cosine query using the recorded query embedding.\nInstall numpy and pyarrow then run python examples/query.py PACK_DIRECTORY.\n")?;
        std::fs::write(
            stage.join("examples/query.py"),
            include_str!("../../../scripts/pack-query.py"),
        )?;
        let vector = engines.vector(query, true)?;
        let mut scores = chunks
            .iter()
            .map(|c| {
                (
                    c.chunk_id.clone(),
                    c.embedding
                        .iter()
                        .zip(&vector)
                        .map(|(a, b)| a * b)
                        .sum::<f32>(),
                )
            })
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let example = json!({"query":query,"query_embedding":vector,"expected_top":scores.iter().take(3).map(|v| &v.0).collect::<Vec<_>>(),"method":"cosine on L2-normalized float32 embeddings; ties by chunk_id","embedding_recipe_sha256":sha256(&serde_json::to_vec(&serde_json::to_value(&engines.recipe)?)?)});
        std::fs::write(
            stage.join("examples/query.json"),
            serde_json::to_string_pretty(&example)? + "\n",
        )?;
        let mut hashes = BTreeMap::new();
        for entry in walkdir::WalkDir::new(&stage) {
            let entry = entry?;
            if entry.file_type().is_file() {
                hashes.insert(
                    entry
                        .path()
                        .strip_prefix(&stage)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    sha256(&std::fs::read(entry.path())?),
                );
            }
        }
        let manifest = json!({"format_version":1,"pack_id":format!("pack-{}",&sha256(serde_json::to_string(&hashes)?.as_bytes())[..24]),"created_at":chrono::Utc::now().to_rfc3339(),"counts":{"documents":docs.len(),"chunks":chunks.len()},"embedding_recipe":engines.recipe,"files":hashes,"hash_scope":"all payload files; manifest.json excludes itself","originals":true});
        std::fs::write(
            stage.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)? + "\n",
        )?;
        verify(&stage)?;
        std::fs::rename(&stage, output)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}
pub fn verify(pack: &Path) -> Result<Value> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(pack.join("manifest.json"))?)?;
    for (name, digest) in manifest["files"].as_object().context("manifest files")? {
        let relative = Path::new(name);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            bail!("unsafe pack path");
        }
        if sha256(&std::fs::read(pack.join(relative))?)
            != digest.as_str().context("hash must be string")?
        {
            bail!("pack hash mismatch: {name}");
        }
    }
    Ok(manifest)
}
/// Environment credentials are used solely inside this explicit operation.
/// reqwest uses native trust roots and system proxy discovery (environment overrides supported).
pub fn upload(pack: &Path, endpoint: &str, bucket: &str, region: &str, prefix: &str) -> Result<()> {
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256};
    let manifest = verify(pack)?;
    let access = std::env::var("AWS_ACCESS_KEY_ID").context("AWS_ACCESS_KEY_ID required")?;
    let secret =
        std::env::var("AWS_SECRET_ACCESS_KEY").context("AWS_SECRET_ACCESS_KEY required")?;
    let session = std::env::var("AWS_SESSION_TOKEN").ok();
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let mut names = manifest["files"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    // Manifest last: incomplete uploads have no final inventory.
    names.push("manifest.json".into());
    for name in names {
        let body = std::fs::read(pack.join(&name))?;
        let mut url = url::Url::parse(endpoint)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("endpoint must be an HTTP(S) origin without credentials or query");
        }
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("endpoint cannot be a base"))?
            .pop_if_empty()
            .push(bucket)
            .extend(prefix.split('/').filter(|s| !s.is_empty()))
            .extend(name.split('/'));
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().context("endpoint host")?),
            None => url.host_str().context("endpoint host")?.to_owned(),
        };
        let date = chrono::Utc::now();
        let day = date.format("%Y%m%d").to_string();
        let timestamp = date.format("%Y%m%dT%H%M%SZ").to_string();
        let hash = sha256(&body);
        let mut headers = BTreeMap::from([
            ("host", host.clone()),
            ("x-amz-content-sha256", hash.clone()),
            ("x-amz-date", timestamp.clone()),
        ]);
        if let Some(token) = &session {
            headers.insert("x-amz-security-token", token.clone());
        }
        let signed = headers.keys().copied().collect::<Vec<_>>().join(";");
        let canonical_headers = headers
            .iter()
            .map(|(k, v)| format!("{k}:{}\n", v.trim()))
            .collect::<String>();
        let canonical = format!(
            "PUT\n{}\n\n{canonical_headers}\n{signed}\n{hash}",
            url.path()
        );
        let scope = format!("{day}/{region}/s3/aws4_request");
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{:x}",
            Sha256::digest(canonical.as_bytes())
        );
        let sign = |key: &[u8], data: &[u8]| -> Vec<u8> {
            let mut hmac =
                Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts all key lengths");
            hmac.update(data);
            hmac.finalize().into_bytes().to_vec()
        };
        let key = sign(format!("AWS4{secret}").as_bytes(), day.as_bytes());
        let key = sign(&key, region.as_bytes());
        let key = sign(&key, b"s3");
        let key = sign(&key, b"aws4_request");
        let signature = sign(&key, to_sign.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let authorization = format!("AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed}, Signature={signature}");
        let mut request = client
            .put(url)
            .body(body)
            .header("authorization", authorization);
        for (key, value) in headers {
            request = request.header(key, value);
        }
        let response = request.send()?;
        if !response.status().is_success() {
            bail!("S3 upload failed for {name}: HTTP {}", response.status());
        }
    }
    Ok(())
}
pub fn recipe(state: &State) -> Result<Recipe> {
    Ok(serde_json::from_slice(&std::fs::read(
        state.root.join("recipe.json"),
    )?)?)
}
