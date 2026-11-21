use serde_json::{json, Value};
use vz_pack::handoff::{
    bucket_selection_digest, validate_lengths, Handoff, MAX_ENCODED_BYTES, MAX_EXPANDED_BYTES,
};

const PACK: &str = "550e8400-e29b-41d4-a716-446655440000";

fn golden_payloads() -> [Value; 3] {
    let draft = json!({
        "title": "Orbital Archive", "brief": "Retention policies", "description": "",
        "category": "Research", "tags": "archive"
    });
    [
        json!({"version":1,"pack_id":PACK,"draft":draft,"provider":"aws",
            "bucket":"seller-archive","region":"eu-west-1","connection_folder":"vz",
            "pack_prefix":format!("vz/{PACK}/"),"file_count":6,"selection_sha256":"1".repeat(64)}),
        json!({"version":1,"pack_id":PACK,"draft":draft,"provider":"r2",
            "bucket":"seller-archive","account_id":"0123456789abcdef","connection_folder":"vz",
            "pack_prefix":format!("vz/{PACK}/"),"file_count":6,"selection_sha256":"2".repeat(64)}),
        json!({"version":1,"pack_id":PACK,"draft":draft,"files":[
            {"size":938,"sha256":"3".repeat(64)},{"size":u64::MAX,"sha256":"4".repeat(64)}
        ]}),
    ]
}

fn native_valid(value: Value) -> bool {
    serde_json::from_value::<Handoff>(value).is_ok_and(|handoff| handoff.validate().is_ok())
}

#[test]
fn golden_payloads_match_backend_handoff_models() {
    // Backend origin/main 1cfcdc604, app/services/listing_authority_import.py:58-114
    // and app/schemas/listing_authority.py:7. Strict v1, canonical UUID, draft
    // string field bounds, disjoint closed shapes, exact bucket prefix and
    // provider-specific metadata, lowercase SHA256, u64 sizes and unique pairs.
    for fixture in golden_payloads() {
        assert!(native_valid(fixture.clone()));
        let handoff: Handoff = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(serde_json::to_value(handoff).unwrap(), fixture);
    }

    let [aws, r2, gateway] = golden_payloads();
    for (fixture, field, value) in [
        (&gateway, "version", json!(true)),
        (&gateway, "version", json!(1.0)),
        (&gateway, "pack_id", json!(PACK.to_uppercase())),
        (&gateway, "pack_id", json!(PACK.replace('-', ""))),
        (&gateway, "files", json!([])),
        (
            &gateway,
            "files",
            json!([{"size":-1,"sha256":"3".repeat(64)}]),
        ),
        (
            &gateway,
            "files",
            json!([{"size":1,"sha256":"A".repeat(64)}]),
        ),
        (
            &gateway,
            "files",
            json!([gateway["files"][0], gateway["files"][0]]),
        ),
        (&aws, "connection_folder", json!("other")),
        (&aws, "pack_prefix", json!("vz/different/")),
        (&aws, "file_count", json!("6")),
        (&aws, "file_count", json!(0)),
        (&aws, "bucket", json!("a".repeat(256))),
        (&aws, "region", json!("a".repeat(81))),
        (&aws, "account_id", json!("unwanted")),
        (&r2, "account_id", json!("a".repeat(129))),
        (&r2, "region", json!("unwanted")),
    ] {
        let mut invalid = fixture.clone();
        invalid[field] = value;
        assert!(!native_valid(invalid), "{field}");
    }
}

#[test]
fn handoff_boundary() {
    assert!(validate_lengths(MAX_ENCODED_BYTES, MAX_EXPANDED_BYTES).is_ok());
    assert!(validate_lengths(MAX_ENCODED_BYTES + 1, 1).is_err());
    assert!(validate_lengths(102, MAX_EXPANDED_BYTES + 1).is_err());
    assert!(serde_json::from_str::<Handoff>("{").is_err());

    let [bucket, _, mut gateway] = golden_payloads();
    for fixture in [&bucket, &gateway] {
        for field in [
            "source_revision",
            "native_commitment",
            "path",
            "url",
            "credentials",
            "draft_id",
            "bucket_files",
            "identity_terms",
            "trusted_client",
        ] {
            let mut forged = fixture.clone();
            forged[field] = json!("caller input");
            assert!(!native_valid(forged), "{field}");
        }
    }
    let pairs: Vec<_> = (0..65)
        .map(|size| json!({"size":size,"sha256":"3".repeat(64)}))
        .collect();
    gateway["files"] = json!(pairs[..64]);
    assert!(native_valid(gateway.clone()));
    gateway["files"] = json!(pairs);
    assert!(!native_valid(gateway));

    let key = format!("vz/{PACK}/manifest.json");
    let original = bucket_selection_digest(PACK, &[(&key, "\"source-etag-2\"", 50)]).unwrap();
    let swapped = bucket_selection_digest(PACK, &[(&key, "\"replacement-etag-2\"", 50)]).unwrap();
    assert_ne!(original, swapped);
}

#[test]
fn managed_deny_precedes_preview_and_store_access() {
    use std::{path::PathBuf, sync::Arc};
    use vz_pack::{
        handoff::get_listing_handoff,
        listing_commands::ListingReference,
        listing_policy::{ListingError, ListingPolicy, ManagedPolicy, PolicySource},
    };
    struct Denied;
    impl PolicySource for Denied {
        fn read(&self) -> ManagedPolicy {
            ManagedPolicy::Denied
        }
    }
    let reference = ListingReference {
        store: PathBuf::new(),
        library: PathBuf::new(),
        current: serde_json::from_value(json!({
            "pack_id":PACK, "generation":PACK, "review_hash":"0".repeat(64)
        }))
        .unwrap(),
    };
    let request = serde_json::from_value(json!({
        "draft":golden_payloads()[0]["draft"], "target":{"type":"gateway","names":[]}
    }))
    .unwrap();
    let error = get_listing_handoff(&ListingPolicy::new(Arc::new(Denied)), &reference, request)
        .unwrap_err();
    assert_eq!(error.code, ListingError::PolicyDenied);
}

#[cfg(all(
    debug_assertions,
    feature = "listing-preview-harness",
    not(feature = "listing-originals"),
    any(target_os = "macos", windows)
))]
#[test]
fn import_marker_idempotency() {
    use std::{fs, path::Path};
    use vz_pack::{
        handoff::get_listing_handoff,
        listing_commands::{review, ListingReference},
        listing_policy::{system, ListingError},
        review_build::rebuild,
        review_store::ReviewStore,
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines = vz_core::engine::Engines::load_embedding(
        &assets,
        &repo.join("spike/fixtures/models.json"),
        true,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let library = dir.path().join("library");
    let store = dir.path().join("reviews");
    fs::create_dir(&source).unwrap();
    let file = source.join("sample.txt");
    fs::write(&file, "Orbital archive retention is thirty days.").unwrap();
    let index = || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(vz_core::active_index::reindex(
                &library,
                &source,
                &engines,
                &assets,
                |_| {},
                || false,
            ))
            .unwrap();
    };
    index();
    let current = rebuild(
        &store,
        &library,
        &source,
        uuid::Uuid::new_v4(),
        &engines,
        "What is the retention policy?",
        &Default::default(),
    )
    .unwrap();
    let reference = ListingReference {
        store,
        library: library.clone(),
        current,
    };
    let request = || {
        serde_json::from_value(json!({
            "draft": golden_payloads()[0]["draft"],
            "target": {"type":"gateway","names":[
                "manifest.json","chunks.parquet","documents.parquet","README.md","query.py","query.json"
            ]}
        })).unwrap()
    };
    let policy = system();
    assert_eq!(
        get_listing_handoff(policy, &reference, request())
            .unwrap_err()
            .code,
        ListingError::ReviewRequired
    );
    let accepted = review(policy, &reference.store, &library, &reference.current, true).unwrap();
    let first = get_listing_handoff(policy, &reference, request()).unwrap();
    let second = get_listing_handoff(policy, &reference, request()).unwrap();
    assert_eq!(first.idempotency_key, reference.current.generation);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );

    let mut forged = reference.clone();
    forged.current.review_hash = "1".repeat(64).try_into().unwrap();
    assert_eq!(
        get_listing_handoff(policy, &forged, request())
            .unwrap_err()
            .code,
        ListingError::StaleGeneration
    );

    // Freeze and accept a legacy-shaped generation using the same active
    // index so the archive is the reason it goes stale.
    let legacy = dir.path().join("legacy");
    let vector_payload = reference
        .store
        .join(reference.current.pack_id.to_string())
        .join(reference.current.generation.to_string())
        .join("payload");
    for artifact in &accepted.staged_artifacts {
        let destination = legacy.join(&artifact.artifact);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(vector_payload.join(&artifact.artifact), destination).unwrap();
    }
    fs::write(legacy.join("originals-0001.zip"), b"legacy original bytes").unwrap();
    let store = ReviewStore::open(&reference.store).unwrap();
    let pack_id = uuid::Uuid::new_v4();
    let binding = store
        .lock_native(policy, pack_id, &library)
        .unwrap()
        .capture(&source)
        .unwrap();
    let guard = store.lock(policy, pack_id).unwrap();
    let mut legacy_review = accepted;
    legacy_review.pack_id = pack_id;
    legacy_review.accepted_review_hash = None;
    let legacy_current = guard.freeze(binding, legacy_review, &legacy).unwrap();
    guard.accept(&legacy_current).unwrap();
    drop(guard);
    let legacy_reference = ListingReference {
        current: legacy_current,
        ..reference.clone()
    };
    assert_eq!(
        get_listing_handoff(policy, &legacy_reference, request())
            .unwrap_err()
            .code,
        ListingError::StaleGeneration
    );

    fs::write(&file, "The new retention policy is sixty days.").unwrap();
    index();
    assert_eq!(
        get_listing_handoff(policy, &reference, request())
            .unwrap_err()
            .code,
        ListingError::StaleGeneration
    );
}
