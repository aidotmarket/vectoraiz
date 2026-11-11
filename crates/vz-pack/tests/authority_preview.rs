//! T2/T3 pack_names_v1 handler roundtrips and policy/default-off boundaries.
//! Native frozen-generation transfer races also run in review_build's tests;
//! the pipeline harness exercises real engines and remote destinations.
use std::sync::{Arc, Mutex};
use vz_pack::listing_policy::{ListingError, ListingPolicy, ManagedPolicy, PolicySource};

fn pack_names_v1() -> (serde_json::Value, Vec<u8>) {
    use std::io::{Cursor, Write};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pack_names_v1.json")).unwrap();
    let mut inner = zip::ZipWriter::new(Cursor::new(Vec::new()));
    inner
        .start_file(
            "private-manifest.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    inner
        .write_all(&serde_json::to_vec(&fixture["purchaser"]).unwrap())
        .unwrap();
    let schema = std::sync::Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "client_name",
        arrow_schema::DataType::Utf8,
        true,
    )]));
    let batch = arrow_array::RecordBatch::try_new(
        schema.clone(),
        vec![std::sync::Arc::new(arrow_array::StringArray::from(vec![
            Some("Project ORION"),
            None,
        ]))],
    )
    .unwrap();
    let mut parquet = parquet::arrow::ArrowWriter::try_new(Vec::new(), schema, None).unwrap();
    parquet.write(&batch).unwrap();
    inner
        .start_file(
            "private-table.parquet",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    inner.write_all(&parquet.into_inner().unwrap()).unwrap();
    let mut outer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    outer
        .start_file(
            "private-container.zip",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    outer
        .write_all(&inner.finish().unwrap().into_inner())
        .unwrap();
    (fixture, outer.finish().unwrap().into_inner())
}

#[test]
fn delivered_inventory_roundtrip() {
    use vz_pack::sanitize::{rebuild_artifact, Changes};
    let (fixture, bytes) = pack_names_v1();
    let mut changes = Changes {
        identity_terms: serde_json::from_value(fixture["identity_terms"].clone()).unwrap(),
        ..Changes::default()
    };
    let location = "originals.zip!document-000001.zip!document-000001.json/title";
    changes
        .edits
        .insert(location.into(), "Reviewed purchaser title".into());
    let result = rebuild_artifact("originals.zip", &bytes, &changes).unwrap();
    assert!(result
        .inventory
        .iter()
        .any(|e| e.location == location && e.value.as_deref() == Some("Reviewed purchaser title")));
    assert!(result
        .inventory
        .iter()
        .any(|e| e.location.ends_with("/date_ranges") && e.value.is_none()));
    assert!(result
        .inventory
        .iter()
        .any(|e| e.location.ends_with("/unmeasured_rows") && e.value.is_none()));
    assert!(result.inventory.iter().any(
        |e| e.location.ends_with("/column/0/name") && e.value.as_deref() == Some("client_name")
    ));
    assert!(result
        .inventory
        .iter()
        .any(|e| e.location.ends_with("/row/0/column/0")
            && e.value.as_deref() == Some("Project ORION")));
    // Rebuilding the delivered result without edits gives the same complete
    // inventory. Independent readback is what acceptance binds, not a summary.
    let readback = rebuild_artifact(
        "originals.zip",
        &result.bytes,
        &Changes {
            identity_terms: changes.identity_terms,
            ..Changes::default()
        },
    )
    .unwrap();
    assert_eq!(readback.inventory, result.inventory);
    assert_ne!(
        vz_core::pipeline::sha256(&bytes),
        vz_core::pipeline::sha256(&result.bytes)
    );
    for planted_private_name in [
        "private-container.zip",
        "private-manifest.json",
        "private-table.parquet",
        "identity_terms",
    ] {
        assert!(!result
            .bytes
            .windows(planted_private_name.len())
            .any(|w| w == planted_private_name.as_bytes()));
    }
    let mut offset = 0;
    let mut locations = Vec::new();
    loop {
        let page = vz_pack::sanitize::page(&result.inventory, "", offset, 7);
        if page.is_empty() {
            break;
        }
        offset += page.len();
        locations.extend(page.into_iter().map(|e| e.location.clone()));
    }
    assert_eq!(locations.len(), result.inventory.len());
}

#[test]
fn identity_and_confidential_names() {
    use vz_pack::sanitize::{check_identifier, rebuild_artifact, Changes};
    let (fixture, bytes) = pack_names_v1();
    let terms = serde_json::from_value(fixture["identity_terms"].clone()).unwrap();
    let result = rebuild_artifact(
        "originals.zip",
        &bytes,
        &Changes {
            identity_terms: terms,
            ..Changes::default()
        },
    )
    .unwrap();
    for text in [
        "Confidential fixture-identity document",
        "Client Atlas",
        "Project ORION",
        "client_name",
    ] {
        assert!(
            result
                .inventory
                .iter()
                .any(|e| e.warning && e.value.as_deref() == Some(text)),
            "missing exact-location warning for {text}"
        );
    }
    for identifier in [
        "fixture-identity.example",
        "neutral-host/fixture-identity-alias",
        "vz/fixture-identity",
    ] {
        assert!(check_identifier(identifier, &["FIXTURE-IDENTITY".into()]).is_err());
    }
    assert!(check_identifier("neutral-storage", &["fixture-identity".into()]).is_ok());
    assert!(rebuild_artifact(
        "originals.zip",
        &bytes,
        &Changes {
            identity_terms: vec!["document-000001".into()],
            ..Changes::default()
        }
    )
    .is_err());
    for unsupported in ["unsupported.pdf", "unsupported.docx", "unsupported.xlsx"] {
        assert!(rebuild_artifact(unsupported, b"unsupported", &Changes::default()).is_err());
    }
}

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
