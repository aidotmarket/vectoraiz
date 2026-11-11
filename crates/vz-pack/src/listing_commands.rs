//! Shared native command boundary. No command can accept a caller's payload
//! path in lieu of a frozen generation, or bypass preview availability.
use crate::{
    listing_policy::{ListingError, ListingPolicy},
    review::ListingReview,
    review_store::{CurrentReview, ReviewStore},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Required binding for packaging and every destination. An arbitrary pack
/// directory or pack UUID is never a substitute for this exact generation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingReference {
    pub store: PathBuf,
    pub library: PathBuf,
    pub current: CurrentReview,
}

/// All synchronous transfer entries retain these guards through destination
/// writes/readback, promotion and durable completion. Only frozen, accepted,
/// sanitizer-certified generations expose a payload to an adapter.
pub(crate) fn with_reviewed<T>(
    reference: Option<&ListingReference>,
    run: impl FnOnce(
        &crate::review_store::NativePackReview<'_>,
        &CurrentReview,
        &Path,
        &[String],
    ) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let policy = crate::listing_policy::system();
    policy.require_preview()?;
    let reference = reference.ok_or(ListingError::ReviewRequired)?;
    let store = ReviewStore::open(&reference.store)?;
    let pack = store.lock_native_current(policy, &reference.current, &reference.library)?;
    let (payload, terms) = pack.reviewed_payload(&reference.current)?;
    run(&pack, &reference.current, &payload, &terms)
}

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
                .unwrap_or(ListingError::OperationFailed),
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
        let pack = store.lock_native_current(policy, current, library)?;
        if accept {
            pack.check_sanitizer(current)?;
            pack.accept(current)
        } else {
            pack.inspect(current)
        }
    };
    run().map_err(Into::into)
}

pub fn invalidate(
    policy: &ListingPolicy,
    reference: &ListingReference,
) -> Result<(), ListingFailure> {
    let run = || -> anyhow::Result<()> {
        policy.require_preview()?;
        let store = ReviewStore::open(&reference.store)?;
        let pack = store.lock_native_current(policy, &reference.current, &reference.library)?;
        pack.begin_rebuild()
    };
    run().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_errors_do_not_claim_the_generation_is_stale() {
        let error = ListingFailure::from(anyhow::anyhow!("destination cannot be read"));
        assert_eq!(error.code, ListingError::OperationFailed);
        let stale = ListingFailure::from(anyhow::Error::from(ListingError::StaleGeneration));
        assert_eq!(stale.code, ListingError::StaleGeneration);
    }
}
