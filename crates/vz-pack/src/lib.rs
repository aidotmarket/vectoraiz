//! Minimal portable vector packs and explicitly requested S3-compatible upload.
pub mod export;
pub mod handoff;
pub mod listing_commands;
pub mod listing_policy;
pub mod review;
pub mod review_build;
pub mod review_store;
mod review_transfer;
mod s3;
pub mod sanitize;
mod vector_payload;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::Path;
use vz_core::{
    engine::{Engines, Recipe},
    pipeline::sha256,
    storage::State,
};

fn embedding_recipe_sha256(recipe: &Recipe) -> Result<String> {
    let mut value = serde_json::to_value(recipe)?;
    value.sort_all_objects();
    Ok(sha256(&serde_json::to_vec(&value)?))
}

pub fn write(
    root: &Path,
    engines: &Engines,
    output: &Path,
    _query: &str,
    current: Option<&listing_commands::ListingReference>,
) -> Result<Value> {
    let library = current.map(|reference| &reference.library);
    listing_commands::with_reviewed(current, |guard, current, payload, terms| {
        anyhow::ensure!(
            root.canonicalize()? == library.context("review library required")?.canonicalize()?,
            "requested library differs from reviewed library"
        );
        let reference = current;
        let review = guard.check_accepted(reference)?;
        anyhow::ensure!(
            embedding_recipe_sha256(&engines.recipe)?
                == String::from(review.embedding_recipe_sha256),
            "embedding recipe mismatch; re-index required"
        );
        sanitize::check_identifier(&current.pack_id.to_string(), terms)?;
        review_transfer::package(guard, current, payload, output)
    })
}
pub fn verify(pack: &Path) -> Result<Value> {
    verify_checked(pack, &|| Ok(()))
}

pub(crate) fn hash_reader(
    mut reader: impl std::io::Read,
    check: &dyn Fn() -> Result<()>,
) -> Result<(u64, String)> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        check()?;
        let count = match reader.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok((size, format!("{:x}", hash.finalize())));
        }
        hash.update(&buffer[..count]);
        size += count as u64;
    }
}

pub(crate) fn verify_checked(pack: &Path, check: &dyn Fn() -> Result<()>) -> Result<Value> {
    check()?;
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
        check()?;
        let (_, hash) = hash_reader(std::fs::File::open(pack.join(relative))?, check)?;
        if hash != digest.as_str().context("hash must be string")? {
            bail!("pack hash mismatch: {name}");
        }
    }
    Ok(manifest)
}
/// Environment credentials are used solely inside this explicit operation.
/// reqwest uses native trust roots and system proxy discovery (environment overrides supported).
pub fn upload(
    pack: &Path,
    endpoint: &str,
    bucket: &str,
    region: &str,
    prefix: &str,
    current: Option<&listing_commands::ListingReference>,
) -> Result<()> {
    listing_commands::with_reviewed(current, |guard, current, payload, terms| {
        review_transfer::check_input(guard, current, pack, payload)?;
        for identifier in [endpoint, bucket, prefix] {
            sanitize::check_identifier(identifier, terms)?;
        }
        s3::upload(guard, current, payload, endpoint, bucket, region, prefix)
    })
}
pub fn recipe(state: &State) -> Result<Recipe> {
    Ok(serde_json::from_slice(&std::fs::read(
        state.root.join("recipe.json"),
    )?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_embedding_recipe_hash_matches_native() {
        let pins =
            serde_json::from_str(include_str!("../../../spike/fixtures/models.json")).unwrap();
        let recipe = Recipe::from_pins(&pins, 384).unwrap();
        assert_eq!(
            embedding_recipe_sha256(&recipe).unwrap(),
            "5b2e0caa8c3f0d5c5f859e365e758cbe3a438e5fe35b308326079a1c3edd42aa"
        );
    }
}
