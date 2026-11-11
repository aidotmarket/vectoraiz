//! Shared native command boundary. No command can accept a caller's payload
//! path in lieu of a frozen generation, or bypass preview availability.
use crate::{
    listing_policy::{ListingError, ListingPolicy},
    review::ListingReview,
    review_store::{CurrentReview, ReviewStore},
};
use serde::Serialize;
use std::path::Path;

/// Additive structured IPC error; stale generations remain distinguishable
/// from managed denial and the default-off feature flag.
#[derive(Debug, Serialize)]
pub struct ListingFailure {
    pub code: ListingError,
    pub reason: String,
}

impl From<anyhow::Error> for ListingFailure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            code: error
                .downcast_ref::<ListingError>()
                .copied()
                .unwrap_or(ListingError::StaleGeneration),
            reason: format!("{error:#}"),
        }
    }
}

impl std::fmt::Display for ListingFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.reason)
    }
}
impl std::error::Error for ListingFailure {}

pub fn review(
    policy: &ListingPolicy,
    store: &Path,
    library: &Path,
    current: &CurrentReview,
    accept: bool,
) -> Result<ListingReview, ListingFailure> {
    let run = || -> anyhow::Result<ListingReview> {
        policy.require_preview()?;
        let store = ReviewStore::open(store)?;
        let pack = store.lock_native(policy, current.pack_id, library)?;
        if accept {
            pack.accept(current)
        } else {
            pack.inspect(current)
        }
    };
    run().map_err(Into::into)
}
