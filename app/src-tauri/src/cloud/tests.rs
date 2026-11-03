use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Condvar, Mutex as StdMutex,
};
#[derive(Default)]
struct FakeStore {
    keys: StdMutex<std::collections::HashMap<Provider, Secret>>,
    fail_load: StdMutex<Option<CredError>>,
    fail_write: AtomicBool,
    fail_remove: AtomicBool,
    pause: StdMutex<Option<Arc<Pause>>>,
}
impl Store for FakeStore {
    fn store(&self, p: Provider, key: &Secret) -> Result<(), CredError> {
        if let Some(pause) = self.pause.lock().unwrap().clone() {
            pause.wait();
        }
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(CredError::Denied);
        }
        // Full fake secrets stay in memory, including during the transport leak scan.
        self.keys.lock().unwrap().insert(p, key.copy_for_test());
        Ok(())
    }
    fn load(&self, p: Provider) -> Result<Secret, CredError> {
        if let Some(error) = *self.fail_load.lock().unwrap() {
            return Err(error);
        }
        self.keys
            .lock()
            .unwrap()
            .get(&p)
            .map(Secret::copy_for_test)
            .ok_or(CredError::Missing)
    }
    fn remove(&self, p: Provider) -> Result<(), CredError> {
        if self.fail_remove.load(Ordering::SeqCst) {
            return Err(CredError::Unavailable);
        }
        self.keys.lock().unwrap().remove(&p);
        Ok(())
    }
}
#[derive(Default)]
struct Pause {
    entered: AtomicBool,
    released: StdMutex<bool>,
    ready: Condvar,
}
impl Pause {
    fn wait(&self) {
        // A real HTTP completion can run on the worker driving timers. Hand
        // that worker back while a synchronous lifecycle hook is paused.
        tokio::task::block_in_place(|| {
            self.entered.store(true, Ordering::SeqCst);
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.ready.wait(released).unwrap();
            }
        });
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.ready.notify_all();
    }
    async fn entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !self.entered.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
}
fn setup(
    hook: impl Fn(&str) + Send + Sync + 'static,
) -> (tempfile::TempDir, Arc<Cloud>, Arc<FakeStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FakeStore::default());
    let cloud = Arc::new(Cloud {
        cloud_lock: Mutex::new(()),
        store: store.clone(),
        hook: Arc::new(hook),
        transport: Transport::new(),
    });
    (dir, cloud, store)
}
async fn consent(c: &Cloud, dir: &Path, p: Provider) {
    c.confirm(dir, p, cloud_gate::disclosure_version())
        .await
        .unwrap();
}
fn fake_key() -> Secret {
    Secret::new("sk-test-SENTINEL-1234".into())
}
async fn set(c: &Cloud, dir: &Path, p: Provider) -> Result<Value, LifecycleError> {
    c.set(dir, p, fake_key()).await
}
fn disk(dir: &Path) -> Value {
    settings::Settings::read(dir).unwrap().0
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_pause_points_stop_cancel_provider_change_and_second_setup() {
    for point in ["key_check", "intent", "store", "before_commit"] {
        for transition in ["stop", "cancel", "provider", "second"] {
            let pause = Arc::new(Pause::default());
            let once = Arc::new(AtomicBool::new(false));
            let hook_pause = pause.clone();
            let hook_once = once.clone();
            let name = point.to_owned();
            let (dir, cloud, store) = setup(move |phase| {
                if phase == name && !hook_once.swap(true, Ordering::SeqCst) {
                    hook_pause.wait();
                }
            });
            if point == "store" {
                *store.pause.lock().unwrap() = Some(pause.clone());
            }
            consent(&cloud, dir.path(), Provider::Anthropic).await;
            let c = cloud.clone();
            let path = dir.path().to_owned();
            let first = tokio::spawn(async move { set(&c, &path, Provider::Anthropic).await });
            pause.entered().await;
            let c = cloud.clone();
            let path = dir.path().to_owned();
            let next = match transition {
                "stop" => Some(tokio::spawn(async move { c.stop(&path).await })),
                "provider" => Some(tokio::spawn(async move {
                    c.confirm(&path, Provider::OpenAi, cloud_gate::disclosure_version())
                        .await
                })),
                "second" => Some(tokio::spawn(async move {
                    set(&c, &path, Provider::Anthropic)
                        .await
                        .map_err(|e| e.to_string())
                })),
                _ => {
                    cancel(dir.path(), "cloud_setup").unwrap();
                    None
                }
            };
            if matches!(transition, "stop" | "provider") {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        if settings::with_state(dir.path(), |s| {
                            s.ops.values().any(|o| o.mark == Mark::Revoked)
                        })
                        .unwrap()
                        {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
            if let Some(next) = &next {
                assert!(!next.is_finished());
            }
            if transition == "second" {
                // API callers queue under cloud_lock; the UI has only one dialog.
                pause.release();
                first.await.unwrap().unwrap();
                next.unwrap().await.unwrap().unwrap();
                continue;
            }
            pause.release();
            let result = first.await.unwrap();
            assert!(
                matches!(
                    result.as_ref().err().map(|e| e.to_string()).as_deref(),
                    Some("cancelled" | "revoked")
                ),
                "{point}/{transition}: {result:?}"
            );
            if let Some(next) = next {
                next.await.unwrap().unwrap();
            }
            assert!(store.keys.lock().unwrap().is_empty());
            assert_ne!(disk(dir.path())["paths"]["api_key"]["enabled"], true);
            if transition == "provider" {
                assert_eq!(
                    disk(dir.path())["paths"]["api_key"]["consent"]["provider"],
                    "openai"
                );
            }
            settings::restart(dir.path());
            assert_ne!(status(dir.path()).unwrap()["api_key"]["enabled"], true);
        }
    }
}
#[tokio::test]
async fn lifecycle_recovery_cleanup_reuse_and_stop() {
    let (dir, c, store) = setup(|_| {});
    let p = Provider::Anthropic;
    assert_eq!(
        set(&c, dir.path(), p).await.unwrap_err().to_string(),
        "consent_required"
    );
    consent(&c, dir.path(), p).await;
    assert_eq!(status_at(dir.path())["consent_version_current"], true);
    let status = set(&c, dir.path(), p).await.unwrap();
    assert_eq!(status["api_key"]["key_last4"], "1234");
    assert_eq!(status["default"], "api_key");
    settings::update(dir.path(), |s| {
        s.settings.api_mut()["pending"] = json!(p);
        Ok::<(), String>(())
    })
    .unwrap();
    settings::restart(dir.path());
    let op = register(dir.path(), "cloud").unwrap();
    assert!(matches!(c.load_for_ask(&op,p).await,Err(e) if e.to_string()=="no_key"));
    drop(op);
    assert!(store.keys.lock().unwrap().is_empty());
    set(&c, dir.path(), p).await.unwrap();
    store.fail_remove.store(true, Ordering::SeqCst);
    let stopped = c.stop(dir.path()).await.unwrap();
    assert_eq!(stopped["cleanup_pending"], true);
    assert_eq!(stopped["message"], cleanup_message(p));
    assert!(!stopped["message"]
        .as_str()
        .unwrap()
        .contains("and removed your key"));
    consent(&c, dir.path(), Provider::OpenAi).await;
    assert_eq!(status_at(dir.path())["consent_provider"], "openai");
    assert_eq!(status_at(dir.path())["consent_version_current"], true);
    assert_eq!(status_at(dir.path())["consent_current"], false);
    assert!(set(&c, dir.path(), Provider::OpenAi).await.is_err());
    store.fail_remove.store(false, Ordering::SeqCst);
    c.retry(dir.path()).await.unwrap();
    set(&c, dir.path(), Provider::OpenAi).await.unwrap();
    let stopped = c.stop(dir.path()).await.unwrap();
    assert_eq!(stopped["cleanup_pending"], false);
    assert_eq!(stopped["message"], "vectorAIz has stopped sending to OpenAI and removed your key from this computer. What was already sent can't be taken back.");
    assert!(store.keys.lock().unwrap().is_empty());
    assert_eq!(
        disk(dir.path())["paths"]["api_key"]["consent"]["provider"],
        "openai"
    );
}
fn status_at(dir: &Path) -> Value {
    status(dir).unwrap()
}
#[tokio::test]
async fn credential_failures_and_missing_repair() {
    let (dir, c, store) = setup(|_| {});
    let p = Provider::Anthropic;
    consent(&c, dir.path(), p).await;
    store.fail_write.store(true, Ordering::SeqCst);
    let failure = lifecycle_failure(set(&c, dir.path(), p).await.unwrap_err(), p);
    assert_eq!(failure["reason"], "credential_unavailable");
    assert_eq!(failure["message"], credential_message(p));
    assert!(store.keys.lock().unwrap().is_empty());
    store.fail_write.store(false, Ordering::SeqCst);
    set(&c, dir.path(), p).await.unwrap();
    for error in [CredError::Denied, CredError::Unavailable] {
        *store.fail_load.lock().unwrap() = Some(error);
        let op = register(dir.path(), "cloud").unwrap();
        let failure = lifecycle_failure(c.load_for_ask(&op, p).await.err().unwrap(), p);
        assert_eq!(failure["reason"], "credential_unavailable");
        assert_eq!(failure["message"], credential_message(p));
        drop(op);
        let result = c
            .ask(dir.path(), "How long?", retrieval(), |_| {
                panic!("credential failure emitted a token")
            })
            .await
            .unwrap();
        assert_eq!(result["reason"], "credential_unavailable");
        assert_eq!(result["answered_by"], Value::Null);
    }
    *store.fail_load.lock().unwrap() = None;
    store.keys.lock().unwrap().clear();
    let op = register(dir.path(), "cloud").unwrap();
    assert!(matches!(c.load_for_ask(&op,p).await,Err(e) if e.to_string()=="no_key"));
    assert_eq!(status_at(dir.path())["api_key"]["key_present"], false);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn commit_order_cancel_and_failed_write_publish_nothing() {
    for (point, write_failure) in [
        ("commit", false),
        ("before_commit", false),
        ("commit", true),
    ] {
        let pause = Arc::new(Pause::default());
        let hook_pause = pause.clone();
        let after_commit = Arc::new(Pause::default());
        let hold_committed = after_commit.clone();
        let (dir, c, store) = setup(move |phase| {
            if phase == point {
                hook_pause.wait();
            }
            if phase == "after_commit" && point == "commit" && !write_failure {
                hold_committed.wait();
            }
        });
        consent(&c, dir.path(), Provider::Anthropic).await;
        settings::update(dir.path(), |s| {
            s.settings.select(Some("local-model"));
            Ok::<(), String>(())
        })
        .unwrap();
        let cc = c.clone();
        let path = dir.path().to_owned();
        let first = tokio::spawn(async move { set(&cc, &path, Provider::Anthropic).await });
        pause.entered().await;
        if write_failure {
            std::fs::remove_file(dir.path().join("settings.json")).unwrap();
            std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        }
        let path = dir.path().to_owned();
        let cancelled = tokio::task::spawn_blocking(move || cancel(&path, "cloud_setup"));
        if point == "commit" {
            assert!(!cancelled.is_finished());
        } else {
            while !settings::with_state(dir.path(), |s| {
                s.ops.values().any(|o| o.mark == Mark::Cancelled)
            })
            .unwrap()
            {
                tokio::task::yield_now().await;
            }
        }
        pause.release();
        // Keep the command alive after commit so the queued cancel observes its
        // committed record before return removes it, as required by the spec.
        if point == "commit" && !write_failure {
            after_commit.entered().await;
        }
        let cancelled = cancelled.await.unwrap();
        after_commit.release();
        let result = first.await.unwrap();
        if point == "commit" && !write_failure {
            assert!(result.is_ok());
            assert_eq!(cancelled.unwrap_err(), "already finished");
            assert!(!store.keys.lock().unwrap().is_empty());
            let status = status_at(dir.path());
            assert_eq!(status["default"], "api_key");
            assert_eq!(status["api_key"]["key_present"], true);
            assert_eq!(disk(dir.path())["paths"]["default"], "api_key");
            assert_eq!(disk(dir.path())["paths"]["api_key"]["key_present"], true);
            assert_eq!(result.unwrap()["answers_switched"], true);
        } else {
            assert!(result.is_err());
            assert_ne!(cancelled.err().as_deref(), Some("already finished"));
            assert!(store.keys.lock().unwrap().is_empty());
            assert_eq!(status_at(dir.path())["api_key"]["key_present"], false);
            assert_eq!(status_at(dir.path())["default"], "local");
        }
    }
}
#[tokio::test]
async fn consent_version_changes_and_defaults_are_memory_only_on_exit() {
    let (dir, c, _) = setup(|_| {});
    revoke(dir.path(), false).unwrap();
    assert!(!dir.path().join("settings.json").exists());
    let p = Provider::Anthropic;
    consent(&c, dir.path(), p).await;
    set(&c, dir.path(), p).await.unwrap();
    let epoch = snapshot(dir.path()).1;
    c.confirm(dir.path(), Provider::OpenAi, "stale".into())
        .await
        .unwrap();
    assert_eq!(snapshot(dir.path()).1, epoch);
    consent(&c, dir.path(), Provider::OpenAi).await;
    assert_eq!(
        set(&c, dir.path(), p).await.unwrap_err().to_string(),
        "consent_required"
    );
    consent(&c, dir.path(), p).await;
    set(&c, dir.path(), p).await.unwrap();
    c.stop(dir.path()).await.unwrap();
    settings::restart(dir.path());
    assert_eq!(status_at(dir.path())["api_key"]["enabled"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_setup_stop_failure_and_unmarked_stopped_branch() {
    let (dir, c, store) = setup(|_| {});
    let p = Provider::Anthropic;
    consent(&c, dir.path(), p).await;
    set(&c, dir.path(), p).await.unwrap();
    let holder = c.cloud_lock.lock().await;
    let cc = c.clone();
    let path = dir.path().to_owned();
    let queued = tokio::spawn(async move { set(&cc, &path, p).await });
    while !settings::with_state(dir.path(), |s| {
        s.ops.values().any(|o| o.kind == "cloud_setup")
    })
    .unwrap()
    {
        tokio::task::yield_now().await;
    }
    let cc = c.clone();
    let path = dir.path().to_owned();
    let stop = tokio::spawn(async move { cc.stop(&path).await });
    while !snapshot(dir.path()).0.stopped {
        tokio::task::yield_now().await;
    }
    std::fs::remove_file(dir.path().join("settings.json")).unwrap();
    std::fs::create_dir(dir.path().join("settings.json")).unwrap();
    drop(holder);
    assert_eq!(queued.await.unwrap().unwrap_err().to_string(), "revoked");
    assert!(stop.await.unwrap().unwrap()["message"]
        .as_str()
        .unwrap()
        .contains("Sending is off until you restart"));
    assert!(store.keys.lock().unwrap().is_empty());
    assert!(snapshot(dir.path()).0.stopped);
    assert!(set(&c, dir.path(), p)
        .await
        .unwrap_err()
        .to_string()
        .contains("Restart vectorAIz"));
    let (dir, c, _) = setup(|_| {});
    consent(&c, dir.path(), p).await;
    revoke(dir.path(), true).unwrap();
    assert_eq!(
        set(&c, dir.path(), p).await.unwrap_err().to_string(),
        "revoked"
    );
    assert_eq!(status_at(dir.path())["stop_failed"], false);
    c.stop(dir.path()).await.unwrap();
    assert!(set(&c, dir.path(), p).await.is_ok());
}
struct ProcessStore(PathBuf);
impl Store for ProcessStore {
    fn store(&self, p: Provider, key: &Secret) -> Result<(), CredError> {
        std::fs::write(self.0.join(p.id()), key.last4()).map_err(|_| CredError::Unavailable)?;
        if std::env::var("VZ_CLOUD_CRASH_PHASE").as_deref() == Ok("store")
            && self.0.join("armed").exists()
        {
            std::process::exit(0);
        }
        Ok(())
    }
    fn load(&self, p: Provider) -> Result<Secret, CredError> {
        std::fs::write(self.0.join("loaded"), b"unexpected load").unwrap();
        std::fs::read_to_string(self.0.join(p.id()))
            .map(Secret::new)
            .map_err(|_| CredError::Missing)
    }
    fn remove(&self, p: Provider) -> Result<(), CredError> {
        match std::fs::remove_file(self.0.join(p.id())) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CredError::Unavailable),
        }
    }
}
#[tokio::test]
async fn crash_intent_and_stored_replacement_reconcile_before_restart_ask() {
    if let Some(root) = std::env::var_os("VZ_CLOUD_CRASH_ROOT") {
        let root = PathBuf::from(root);
        let dir = root.join("models");
        let vault = root.join("vault");
        let hook_vault = vault.clone();
        let c = Cloud {
            cloud_lock: Mutex::new(()),
            store: Arc::new(ProcessStore(vault.clone())),
            transport: Transport::connect(Path::new(
                &std::env::var_os("VZ_CLOUD_CRASH_TRANSPORT").unwrap(),
            )),
            hook: Arc::new(move |phase| {
                if phase == "intent"
                    && std::env::var("VZ_CLOUD_CRASH_PHASE").as_deref() == Ok("intent")
                    && hook_vault.join("armed").exists()
                {
                    std::process::exit(0);
                }
            }),
        };
        if std::env::var("VZ_CLOUD_CRASH_PHASE").as_deref() == Ok("recover") {
            let op = register(&dir, "cloud").unwrap();
            assert!(
                matches!(c.load_for_ask(&op,Provider::Anthropic).await,Err(e) if e.to_string()=="no_key")
            );
            assert!(!vault.join("loaded").exists());
            assert!(!vault.join("anthropic").exists());
            assert_eq!(status_at(&dir)["api_key"]["key_present"], false);
            assert_eq!(status_at(&dir)["api_key"]["key_last4"], Value::Null);
            return;
        }
        consent(&c, &dir, Provider::Anthropic).await;
        set(&c, &dir, Provider::Anthropic).await.unwrap();
        std::fs::write(vault.join("armed"), b"ready").unwrap();
        c.set(
            &dir,
            Provider::Anthropic,
            Secret::new("sk-test-replacement-5678".into()),
        )
        .await
        .unwrap();
        panic!("crash hook did not terminate child");
    }
    for phase in ["intent", "store"] {
        // The surviving parent owns the server. A server spawned in the crash
        // child survives process::exit (no Drop) and on Windows can retain the
        // child's captured pipe handles, so Command::output never reaches EOF.
        let transport = Transport::new();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("models")).unwrap();
        std::fs::create_dir(root.path().join("vault")).unwrap();
        let child = |phase: &str| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "cloud::tests::crash_intent_and_stored_replacement_reconcile_before_restart_ask",
                    "--nocapture",
                ])
                .env("VZ_CLOUD_CRASH_ROOT", root.path())
                .env("VZ_CLOUD_CRASH_PHASE", phase)
                .env("VZ_CLOUD_CRASH_TRANSPORT", &transport.root)
                .output()
                .unwrap();
            assert!(!String::from_utf8_lossy(&output.stdout).contains("sk-test-"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("sk-test-"));
            assert!(
                output.status.success(),
                "crash/recovery child failed ({phase}): {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        child(phase);
        assert_eq!(transport.requests().len(), 2);
        assert_eq!(
            disk(&root.path().join("models"))["paths"]["api_key"]["pending"],
            "anthropic"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("vault/anthropic")).unwrap(),
            if phase == "intent" { "1234" } else { "5678" }
        );
        child("recover");
        assert_eq!(transport.requests().len(), 2, "recovery must not send HTTP");
        assert_eq!(
            disk(&root.path().join("models"))["paths"]["api_key"]["pending"],
            Value::Null
        );
    }
}

/// Owns a real TLS fixture for the lifetime of each lifecycle instance.
pub struct Transport {
    pub client: cloud_gate::TestTransport,
    child: Option<std::process::Child>,
    root: PathBuf,
    _owner: Option<tempfile::TempDir>,
}
impl Transport {
    pub fn new() -> Self {
        use std::io::BufRead;
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("plan.json"), r#"[{"lifecycle":true}]"#).unwrap();
        let mut child =
            std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../scripts/models-test-server.py"),
                )
                .arg(root.path())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
        let mut port = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        std::fs::write(root.path().join("port"), port).unwrap();
        let mut transport = Self::connect(root.path());
        transport.child = Some(child);
        transport._owner = Some(root);
        transport
    }
    fn connect(root: &Path) -> Self {
        let port = std::fs::read_to_string(root.join("port")).unwrap();
        let addr = format!("127.0.0.1:{}", port.trim()).parse().unwrap();
        let client =
            cloud_gate::TestTransport::new(addr, &std::fs::read(root.join("cert.pem")).unwrap());
        Self {
            client,
            child: None,
            root: root.to_owned(),
            _owner: None,
        }
    }
    fn requests(&self) -> Vec<Value> {
        std::fs::read(self.root.join("requests.json"))
            .ok()
            .map(|b| serde_json::from_slice(&b).unwrap())
            .unwrap_or_default()
    }
    async fn closed(&self) {
        wait_for(|| self.root.join("closed").exists()).await;
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
async fn wait_for(check: impl Fn() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !check() {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("lifecycle boundary timed out");
}
fn retrieval() -> impl std::future::Future<Output = Result<Retrieved, String>> {
    std::future::ready(Ok((
        "[1] Thirty days".into(),
        vec![vz_core::index::Citation {
            number: 1,
            doc_id: "doc".into(),
            page: 1,
            chunk_id: "chunk".into(),
        }],
        std::collections::HashMap::new(),
    )))
}
#[tokio::test]
async fn combined_stop_write_and_delete_failure_retry_and_restart() {
    let (dir, c, store) = setup(|_| {});
    let p = Provider::Anthropic;
    consent(&c, dir.path(), p).await;
    set(&c, dir.path(), p).await.unwrap();
    let original = std::fs::read(dir.path().join("settings.json")).unwrap();
    std::fs::remove_file(dir.path().join("settings.json")).unwrap();
    std::fs::create_dir(dir.path().join("settings.json")).unwrap();
    store.fail_remove.store(true, Ordering::SeqCst);
    let stopped = c.stop(dir.path()).await.unwrap();
    assert_eq!(stopped["cleanup_pending"], true);
    let message = stopped["message"].as_str().unwrap();
    assert!(message.contains("Sending is off until you restart."));
    assert!(message.ends_with(&cleanup_message(p)));
    assert_eq!(status(dir.path()).unwrap()["message"], cleanup_message(p));
    assert!(snapshot(dir.path()).0.stopped);
    std::fs::remove_dir(dir.path().join("settings.json")).unwrap();
    std::fs::write(dir.path().join("settings.json"), original).unwrap();
    store.fail_remove.store(false, Ordering::SeqCst);
    assert_eq!(c.retry(dir.path()).await.unwrap()["cleanup_pending"], false);
    assert!(store.keys.lock().unwrap().is_empty());
    settings::restart(dir.path());
    let op = register(dir.path(), "cloud").unwrap();
    assert!(matches!(c.load_for_ask(&op,p).await,Err(e) if e.to_string()=="no_key"));
    assert_eq!(status(dir.path()).unwrap()["api_key"]["key_present"], false);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_ask_boundaries_stop_provider_and_replacement() {
    for p in [Provider::Anthropic, Provider::OpenAi] {
        for boundary in [
            "before_dispatch",
            "mid_stream",
            "before_token",
            "before_final",
        ] {
            for transition in ["stop", "provider", "replace"] {
                let pause = Arc::new(Pause::default());
                let hook_pause = pause.clone();
                let once = Arc::new(AtomicBool::new(false));
                let (dir, c, _) = setup(move |phase| {
                    if phase == boundary && !once.swap(true, Ordering::SeqCst) {
                        hook_pause.wait();
                    }
                });
                consent(&c, dir.path(), p).await;
                set(&c, dir.path(), p).await.unwrap();
                if boundary == "mid_stream" {
                    std::fs::write(c.transport.root.join("stall"), b"").unwrap();
                }
                let tokens = Arc::new(StdMutex::new(Vec::<String>::new()));
                let emitted = tokens.clone();
                let cc = c.clone();
                let path = dir.path().to_owned();
                let ask = tokio::spawn(async move {
                    cc.ask(&path, "How long?", retrieval(), |t| {
                        emitted.lock().unwrap().push(t.into())
                    })
                    .await
                });
                if boundary == "mid_stream" {
                    wait_for(|| !tokens.lock().unwrap().is_empty()).await;
                    wait_for(|| c.transport.root.join("streaming").exists()).await;
                } else {
                    pause.entered().await;
                }
                let before = tokens.lock().unwrap().len();
                match transition {
                    "stop" => {
                        c.stop(dir.path()).await.unwrap();
                    }
                    "provider" => {
                        consent(
                            &c,
                            dir.path(),
                            if p == Provider::Anthropic {
                                Provider::OpenAi
                            } else {
                                Provider::Anthropic
                            },
                        )
                        .await
                    }
                    _ => {
                        set(&c, dir.path(), p).await.unwrap();
                    }
                }
                if boundary == "mid_stream" {
                    c.transport.closed().await;
                }
                pause.release();
                let result = tokio::time::timeout(std::time::Duration::from_secs(5), ask)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(result["kind"], "cancelled", "{boundary}/{transition}");
                assert_eq!(result["answered_by"], Value::Null);
                assert_eq!(
                    tokens.lock().unwrap().len(),
                    before,
                    "late token {boundary}/{transition}"
                );
                let requests = c.transport.requests();
                let asks = requests.iter().filter(|r| r["method"] == "POST").count();
                assert_eq!(
                    asks,
                    usize::from(boundary != "before_dispatch"),
                    "{boundary}/{transition}"
                );
                assert_eq!(
                    requests.len(),
                    1 + asks + usize::from(transition == "replace")
                );
                if asks > 0 {
                    c.transport.closed().await;
                }
            }
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sentinel_production_set_ask_stop_process_scan() {
    const VAR: &str = "VZ_CLOUD_SENTINEL_ROOT";
    if let Some(root) = std::env::var_os(VAR) {
        let root = PathBuf::from(root);
        let models = root.join("separate-model-folder");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        let sentinel = std::env::var("VZ_CLOUD_SENTINEL_FAKE_KEY").unwrap();
        let (_, c, _) = setup(|_| {});
        let p = Provider::Anthropic;
        consent(&c, &models, p).await;
        c.set(&models, p, Secret::new(sentinel.clone()))
            .await
            .unwrap();
        let result = c
            .ask(&models, "How long?", retrieval(), |_| {})
            .await
            .unwrap();
        assert_eq!(result["kind"], "answer");
        assert_eq!(result["text"], "Thirty days [1].\n\nRetrieved sources: [1]");
        assert_eq!(result["answered_by"], "anthropic");
        assert_eq!(c.transport.requests().len(), 2);
        c.transport.closed().await;
        c.stop(&models).await.unwrap();
        // Headers intentionally aren't recorded. Scan actual received bodies.
        std::fs::write(
            root.join("server-bodies.json"),
            serde_json::to_vec(&c.transport.requests()).unwrap(),
        )
        .unwrap();
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let sentinel = format!("sk-test-SENTINEL-{}", uuid::Uuid::new_v4());
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cloud::tests::sentinel_production_set_ask_stop_process_scan",
            "--nocapture",
        ])
        .env(VAR, root.path())
        .env("VZ_CLOUD_SENTINEL_FAKE_KEY", &sentinel)
        .output()
        .unwrap();
    for bytes in [&output.stdout, &output.stderr] {
        assert!(
            !String::from_utf8_lossy(bytes).contains(&sentinel),
            "sentinel leaked to process output"
        );
    }
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fn scan(path: &Path, sentinel: &str) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, sentinel);
            } else {
                assert!(
                    !String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains(sentinel),
                    "sentinel leaked to {}",
                    path.display()
                );
            }
        }
    }
    scan(root.path(), &sentinel);
}

#[tokio::test]
async fn real_key_check_rejection_stores_nothing() {
    let (dir, c, store) = setup(|_| {});
    let p = Provider::Anthropic;
    consent(&c, dir.path(), p).await;
    let result = c
        .set(dir.path(), p, Secret::new("sk-test-reject".into()))
        .await
        .unwrap_err();
    let result = lifecycle_failure(result, p);
    assert_eq!(result["message"], "That key was not accepted.");
    assert_eq!(result["reason"], "key_rejected");
    assert!(store.keys.lock().unwrap().is_empty());
    assert_eq!(c.transport.requests().len(), 1);
}

// Keep the old sender alive but delay its notification, so the dispatch epoch
// check is tested independently of the watch receiver and current consent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dispatch_epoch_guard_without_cancel_delivery() {
    let pause = Arc::new(Pause::default());
    let paused = pause.clone();
    let (dir, c, _) = setup(move |phase| {
        if phase == "before_dispatch" {
            paused.wait();
        }
    });
    let p = Provider::Anthropic;
    consent(&c, dir.path(), p).await;
    set(&c, dir.path(), p).await.unwrap();
    let cc = c.clone();
    let path = dir.path().to_owned();
    let ask = tokio::spawn(async move {
        cc.ask(&path, "How long?", retrieval(), |_| panic!("late token"))
            .await
    });
    pause.entered().await;
    let held_sender = settings::with_state(dir.path(), |s| {
        let op = s.ops.values_mut().find(|o| o.kind == "cloud").unwrap();
        std::mem::replace(&mut op.cancel, watch::channel(false).0)
    })
    .unwrap();
    set(&c, dir.path(), p).await.unwrap();
    pause.release();
    assert_eq!(ask.await.unwrap().unwrap()["kind"], "cancelled");
    assert_eq!(
        c.transport
            .requests()
            .iter()
            .filter(|r| r["method"] == "POST")
            .count(),
        0,
        "dispatch epoch guard allowed a stale request"
    );
    drop(held_sender);
}

#[tokio::test]
async fn every_key_save_switches_local_answers_and_announces_only_changes() {
    let (dir, c, _) = setup(|_| {});
    for p in [Provider::Anthropic, Provider::Anthropic, Provider::OpenAi] {
        consent(&c, dir.path(), p).await;
        settings::update(dir.path(), |s| {
            s.settings.select(Some("local-model"));
            s.settings.0["paths"]["default"] = json!("local");
            Ok::<(), String>(())
        })
        .unwrap();
        let saved = set(&c, dir.path(), p).await.unwrap();
        assert_eq!(saved["default"], "api_key");
        assert_eq!(saved["api_key"]["key_present"], true);
        assert_eq!(saved["answers_switched"], true);
        assert_eq!(
            set(&c, dir.path(), p).await.unwrap()["answers_switched"],
            false
        );
    }
}
