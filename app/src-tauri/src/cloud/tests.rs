use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Condvar, Mutex as StdMutex,
};
#[derive(Default)]
struct FakeStore {
    keys: StdMutex<std::collections::HashMap<Provider, String>>,
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
        // The double keeps only fake-key sentinels; it never touches the OS store.
        self.keys.lock().unwrap().insert(p, key.last4());
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
            .map(|v| Secret::new(v.clone()))
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
        self.entered.store(true, Ordering::SeqCst);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.ready.wait(released).unwrap();
        }
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
async fn set(c: &Cloud, dir: &Path, p: Provider) -> Result<Value, String> {
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
                    set(&c, &path, Provider::Anthropic).await
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
                    result.as_ref().err().map(String::as_str),
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
        set(&c, dir.path(), p).await.unwrap_err(),
        "consent_required"
    );
    consent(&c, dir.path(), p).await;
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
    assert!(matches!(c.load_for_ask(&op,p).await,Err(e) if e=="no_key"));
    drop(op);
    assert!(store.keys.lock().unwrap().is_empty());
    set(&c, dir.path(), p).await.unwrap();
    store.fail_remove.store(true, Ordering::SeqCst);
    c.stop(dir.path()).await.unwrap();
    assert_eq!(status_at(dir.path())["cleanup_pending"], true);
    consent(&c, dir.path(), Provider::OpenAi).await;
    assert!(set(&c, dir.path(), Provider::OpenAi).await.is_err());
    store.fail_remove.store(false, Ordering::SeqCst);
    c.retry(dir.path()).await.unwrap();
    set(&c, dir.path(), Provider::OpenAi).await.unwrap();
    c.stop(dir.path()).await.unwrap();
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
    assert!(set(&c, dir.path(), p)
        .await
        .unwrap_err()
        .contains("password store"));
    assert!(store.keys.lock().unwrap().is_empty());
    store.fail_write.store(false, Ordering::SeqCst);
    set(&c, dir.path(), p).await.unwrap();
    for error in [CredError::Denied, CredError::Unavailable] {
        *store.fail_load.lock().unwrap() = Some(error);
        let op = register(dir.path(), "cloud").unwrap();
        assert!(matches!(c.load_for_ask(&op,p).await,Err(e) if e.contains("password store")));
    }
    *store.fail_load.lock().unwrap() = None;
    store.keys.lock().unwrap().clear();
    let op = register(dir.path(), "cloud").unwrap();
    assert!(matches!(c.load_for_ask(&op,p).await,Err(e) if e=="no_key"));
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
        } else {
            assert!(result.is_err());
            assert_ne!(cancelled.err().as_deref(), Some("already finished"));
            assert!(store.keys.lock().unwrap().is_empty());
            assert_eq!(status_at(dir.path())["api_key"]["key_present"], false);
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
        set(&c, dir.path(), p).await.unwrap_err(),
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
    assert_eq!(queued.await.unwrap().unwrap_err(), "revoked");
    assert!(stop.await.unwrap().unwrap()["message"]
        .as_str()
        .unwrap()
        .contains("Sending is off until you restart"));
    assert!(store.keys.lock().unwrap().is_empty());
    assert!(snapshot(dir.path()).0.stopped);
    assert!(set(&c, dir.path(), p)
        .await
        .unwrap_err()
        .contains("Restart vectorAIz"));
    let (dir, c, _) = setup(|_| {});
    consent(&c, dir.path(), p).await;
    revoke(dir.path(), true).unwrap();
    assert_eq!(set(&c, dir.path(), p).await.unwrap_err(), "revoked");
    assert_eq!(status_at(dir.path())["stop_failed"], false);
    c.stop(dir.path()).await.unwrap();
    assert!(set(&c, dir.path(), p).await.is_ok());
}
#[tokio::test]
async fn every_transition_interrupts_ask_epoch_and_own_receiver() {
    for boundary in ["before_dispatch", "mid_stream", "before_final"] {
        for transition in ["stop", "provider", "replace"] {
            let (dir, c, _) = setup(|_| {});
            let p = Provider::Anthropic;
            consent(&c, dir.path(), p).await;
            set(&c, dir.path(), p).await.unwrap();
            let op = register(dir.path(), "cloud").unwrap();
            let epoch = op.epoch;
            match transition {
                "stop" => {
                    c.stop(dir.path()).await.unwrap();
                }
                "provider" => consent(&c, dir.path(), Provider::OpenAi).await,
                _ => {
                    set(&c, dir.path(), p).await.unwrap();
                }
            }
            assert_ne!(snapshot(dir.path()).1, epoch, "{boundary}/{transition}");
            assert!(*op.cancel.borrow(), "{boundary}/{transition}");
        }
    }
}

#[tokio::test]
async fn sentinel_is_absent_from_all_written_files_and_provider_body() {
    let (dir, c, _) = setup(|_| {});
    let p = Provider::Anthropic;
    consent(&c, dir.path(), p).await;
    let sentinel = format!("sk-test-SENTINEL-{}", uuid::Uuid::new_v4());
    c.set(dir.path(), p, Secret::new(sentinel.clone()))
        .await
        .unwrap();
    let op = register(dir.path(), "cloud").unwrap();
    let key = c.load_for_ask(&op, p).await.unwrap();
    assert_eq!(
        key.last4(),
        sentinel
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
    );
    drop(op);
    let body = cloud_gate::request(p, "fixture question", "[1] fixture passage").unwrap();
    assert!(!body.to_string().contains(&sentinel));
    c.stop(dir.path()).await.unwrap();
    fn scan(path: &Path, sentinel: &str) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, sentinel);
            } else {
                assert!(!String::from_utf8_lossy(&std::fs::read(path).unwrap()).contains(sentinel));
            }
        }
    }
    scan(dir.path(), &sentinel);
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
            assert!(matches!(c.load_for_ask(&op,Provider::Anthropic).await,Err(e) if e=="no_key"));
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
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("models")).unwrap();
        std::fs::create_dir(root.path().join("vault")).unwrap();
        let child = |phase: &str| {
            let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","cloud::tests::crash_intent_and_stored_replacement_reconcile_before_restart_ask","--nocapture"]).env("VZ_CLOUD_CRASH_ROOT",root.path()).env("VZ_CLOUD_CRASH_PHASE",phase).output().unwrap();
            assert!(!String::from_utf8_lossy(&output.stdout).contains("sk-test-"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("sk-test-"));
            assert!(
                output.status.success(),
                "crash/recovery child failed ({phase}): {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        child(phase);
        assert_eq!(
            disk(&root.path().join("models"))["paths"]["api_key"]["pending"],
            "anthropic"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("vault/anthropic")).unwrap(),
            if phase == "intent" { "1234" } else { "5678" }
        );
        child("recover");
        assert_eq!(
            disk(&root.path().join("models"))["paths"]["api_key"]["pending"],
            Value::Null
        );
    }
}
