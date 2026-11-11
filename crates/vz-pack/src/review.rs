//! Local seller-attested review contracts. These are opaque native commitments,
//! not server-verified source facts or proof of an honest client. Persistence is
//! in `review_store`; these contracts alone grant no transfer capability.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Only this schema is understood; unknown versions fail during deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewVersion {
    #[serde(rename = "listing-review-v1")]
    V1,
}

/// A lowercase hexadecimal SHA256 commitment, with validated wire syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256(String);

impl TryFrom<String> for Sha256 {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("expected 64 lowercase hexadecimal SHA256 characters");
        }
        Ok(Self(value))
    }
}

impl From<Sha256> for String {
    fn from(value: Sha256) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedArtifact {
    /// Identifier within the private staged generation; never a handoff field.
    pub artifact: String,
    pub size: u64,
    pub sha256: Sha256,
}

/// A private local inventory. Source/name mappings and identity terms belong in
/// separate private state, never in handoff payloads or delivered artifacts.
/// C1 must bind acceptance and completed transfer to the immutable generation;
/// merely constructing or decoding this record grants no authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingReview {
    pub schema_version: ReviewVersion,
    /// Canonical lowercase hyphenated UUID, also used by the manifest and
    /// handoff pack_id string. New C1 packs replace the Phase 0 pack-<24 hex> id.
    pub pack_id: Uuid,
    pub staged_artifacts: Vec<StagedArtifact>,
    pub metadata_inventory_schema_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata_inventory: Vec<crate::sanitize::Entry>,
    pub source_revision: String,
    pub index_revision: String,
    pub embedding_recipe_sha256: Sha256,
    pub warnings: Vec<String>,
    pub exclusions: Vec<String>,
    pub accepted_review_hash: Option<Sha256>,
    pub completed_transfer_fingerprint: Option<Sha256>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> serde_json::Value {
        json!({
            "schema_version": "listing-review-v1",
            "pack_id": "550e8400-e29b-41d4-a716-446655440000",
            "staged_artifacts": [{"artifact": "documents.parquet", "size": 42, "sha256": "a".repeat(64)}],
            "metadata_inventory_schema_version": 1,
            "source_revision": "source-1", "index_revision": "index-1",
            "embedding_recipe_sha256": "b".repeat(64),
            "warnings": ["Confidential heading at document 1"], "exclusions": ["unsupported.pdf"],
            "accepted_review_hash": "c".repeat(64),
            "completed_transfer_fingerprint": "d".repeat(64)
        })
    }

    #[test]
    fn review_roundtrip() {
        let value = record();
        let record: ListingReview = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(record).unwrap(), value);
    }

    #[test]
    fn uuid_pack_id_is_identical_in_review_and_both_handoff_contracts() {
        use crate::handoff::{bucket_selection_digest, BucketHandoff, GatewayHandoff};
        let review: ListingReview = serde_json::from_value(record()).unwrap();
        let id = review.pack_id.to_string();
        let draft = json!({"title":"Pack", "brief":"", "description":"", "category":"", "tags":""});
        let gateway: GatewayHandoff = serde_json::from_value(json!({
            "version":1, "pack_id":id, "files":[{"size":42,"sha256":"a".repeat(64)}], "draft":draft
        }))
        .unwrap();
        gateway.validate().unwrap();
        let key = format!("vz/{id}/manifest.json");
        let digest = bucket_selection_digest(&id, &[(key.as_str(), "etag", 42)]).unwrap();
        let bucket: BucketHandoff = serde_json::from_value(json!({
            "version":1, "pack_id":id, "provider":"aws", "bucket":"neutral-bucket", "region":"eu-west-1",
            "connection_folder":"vz", "pack_prefix":format!("vz/{id}/"), "file_count":1,
            "selection_sha256":digest, "draft":draft
        })).unwrap();
        bucket.validate().unwrap();
        assert_eq!(gateway.pack_id, id);
        assert_eq!(bucket.pack_id, id);
        assert_eq!(serde_json::to_value(review).unwrap()["pack_id"], id);
    }

    #[test]
    fn unknown_version_and_fields_fail_closed() {
        for version in [json!("listing-review-v2"), json!(1), json!(null)] {
            let mut value = record();
            value["schema_version"] = version;
            assert!(serde_json::from_value::<ListingReview>(value).is_err());
        }
        let mut value = record();
        value["trusted_client"] = json!(true);
        assert!(serde_json::from_value::<ListingReview>(value).is_err());
    }

    #[test]
    fn invalid_uuid_size_and_hash_fail() {
        for (field, invalid) in [
            ("pack_id", json!("pack-not-a-uuid")),
            ("embedding_recipe_sha256", json!("xyz")),
        ] {
            let mut value = record();
            value[field] = invalid;
            assert!(serde_json::from_value::<ListingReview>(value).is_err());
        }
        let mut value = record();
        value["staged_artifacts"][0]["size"] = json!(-1);
        assert!(serde_json::from_value::<ListingReview>(value).is_err());
    }
}
