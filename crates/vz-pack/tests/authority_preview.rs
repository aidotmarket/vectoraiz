//! First C1 slice: policy/cancellation and fail-closed production entry points.
//! Artifact T2/T3 fixtures remain prerequisites for the disabled review journey.
use std::sync::{Arc, Mutex};
use vz_pack::listing_policy::{ListingError, ListingPolicy, ManagedPolicy, PolicySource};

struct Source(Mutex<ManagedPolicy>);
impl PolicySource for Source {
    fn read(&self) -> ManagedPolicy {
        *self.0.lock().unwrap()
    }
}

#[test]
fn managed_policy_cancels_listing() {
    let source = Arc::new(Source(Mutex::new(ManagedPolicy::Unmanaged)));
    let policy = Arc::new(ListingPolicy::new(source.clone()));
    let first = policy.policy_lease().unwrap();
    let second = policy.policy_lease().unwrap();
    assert!(first.check(&policy).is_ok());
    let token = first.cancellation_token();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let b = barrier.clone();
    let p = policy.clone();
    let s = source.clone();
    let update = std::thread::spawn(move || {
        b.wait();
        *s.0.lock().unwrap() = ManagedPolicy::Denied;
        // Policy update takes no pack mutex and must not wait for a listing job.
        p.refresh();
    });
    barrier.wait();
    update.join().unwrap();
    assert!(token.is_cancelled());
    assert!(second.cancellation_token().is_cancelled());
    assert_eq!(first.check(&policy), Err(ListingError::StaleGeneration));
    assert_eq!(policy.require_listing(), Err(ListingError::PolicyDenied));
    *source.0.lock().unwrap() = ManagedPolicy::Allowed;
    policy.refresh();
    assert_eq!(first.check(&policy), Err(ListingError::StaleGeneration));
    assert!(policy.policy_lease().unwrap().check(&policy).is_ok());
    assert_eq!(policy.require_listing(), Err(ListingError::PreviewDisabled));
}

#[test]
fn entry_refresh_catches_unnotified_policy_and_invalid_never_means_unmanaged() {
    let source = Arc::new(Source(Mutex::new(ManagedPolicy::Allowed)));
    let policy = ListingPolicy::new(source.clone());
    let lease = policy.policy_lease().unwrap();
    *source.0.lock().unwrap() = ManagedPolicy::Invalid;
    assert_eq!(
        policy.require_listing(),
        Err(ListingError::PolicyUnreadable)
    );
    assert!(lease.cancellation_token().is_cancelled());
    assert!(policy.policy_lease().is_err());
    // Restart reads installed policy immediately, without a cached allow.
    let restarted = ListingPolicy::new(source);
    assert_eq!(
        restarted.require_listing(),
        Err(ListingError::PolicyUnreadable)
    );
    assert!(restarted.status().reason.contains("Contact IT"));
}

#[test]
fn listing_entries_refuse_before_local_io_credentials_or_network() {
    let absent = std::env::temp_dir().join(format!("vz-c1-absent-{}", std::process::id()));
    assert!(!absent.exists());
    let current = vz_pack::review_store::CurrentReview {
        pack_id: uuid::Uuid::new_v4(),
        generation: uuid::Uuid::new_v4(),
        review_hash: "a".repeat(64).try_into().unwrap(),
    };
    let reference = vz_pack::listing_commands::ListingReference {
        store: absent.join("reviews"),
        library: absent.join("library"),
        current: current.clone(),
    };
    for error in [
        vz_pack::upload(
            &absent,
            "http://127.0.0.1:1",
            "bucket",
            "region",
            "prefix",
            None,
        )
        .unwrap_err(),
        vz_pack::export::folder(&absent, "pack", &absent, &absent, None).unwrap_err(),
        vz_pack::export::folder(&absent, "pack", &absent, &absent, Some(&reference)).unwrap_err(),
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(vz_pack::export::sftp(
                &absent,
                "pack",
                &vz_pack::export::SftpOptions {
                    target: "fixture@127.0.0.1:1:/source".into(),
                    staging: "/staging".into(),
                    identity: Some(absent.clone()),
                    known_hosts: Some(absent.clone()),
                },
                None,
            ))
            .unwrap_err(),
    ] {
        assert!(error.downcast_ref::<ListingError>().is_some(), "{error:#}");
    }
    for accept in [false, true] {
        let failure = vz_pack::listing_commands::review(
            vz_pack::listing_policy::system(),
            &reference.store,
            &reference.library,
            &current,
            accept,
        )
        .unwrap_err();
        assert!(matches!(
            failure.code,
            ListingError::PreviewDisabled
                | ListingError::PolicyDenied
                | ListingError::PolicyUnreadable
        ));
        let wire = serde_json::to_value(failure).unwrap();
        assert!(wire["code"].is_string());
        assert!(wire["reason"].is_string());
    }
    assert!(!absent.exists());
}
