//! Race tests exercise the production gate, settings transactions and real local streams.
use crate::{
    mcp_apps::Apps,
    mcp_channel::{self, Delivery, Request},
    mcp_tools, models, settings,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Engine;
impl models::CachedEngine for Engine {
    fn ensure_chat(&self) -> anyhow::Result<()> {
        panic!("MCP must never load an answer model")
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    dir: PathBuf,
    root: PathBuf,
    apps: Arc<Apps>,
    ops: Arc<models::OperationState<Engine>>,
    id: String,
    opens: Arc<AtomicUsize>,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("models");
        let root = temp.path().join("index");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("recipe.json"), "{}").unwrap();
        let apps = Arc::new(Apps::default());
        let client = apps
            .connect(
                &dir,
                "other",
                "Test",
                &vz_core::cloud_gate::app_disclosure_version(),
            )
            .await
            .unwrap();
        Self {
            _temp: temp,
            dir,
            root,
            apps,
            ops: Arc::new(Default::default()),
            id: client["client_id"].as_str().unwrap().into(),
            opens: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn request(&self, tool: &str) -> Request {
        Request {
            v: 1,
            client: self.id.clone(),
            tool: tool.into(),
            args: match tool {
                "search" => json!({"query":"hello"}),
                "get_document" => json!({"doc_id":"d"}),
                "get_chunk" => json!({"chunk_id":"c"}),
                _ => json!({}),
            },
        }
    }
    fn fail_writes(&self) {
        std::fs::remove_file(self.dir.join("settings.json")).unwrap();
        std::fs::create_dir(self.dir.join("settings.json")).unwrap();
    }
    fn fix_writes(&self) {
        std::fs::remove_dir(self.dir.join("settings.json")).unwrap();
    }
    async fn call(&self, r: Request) -> Value {
        let opens = self.opens.clone();
        mcp_tools::call(
            &self.dir,
            &self.root,
            &self.ops,
            &self.apps,
            r,
            CancellationToken::new(),
            move |_, _| {
                opens.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"hits":[{"text":"private"}]}))
            },
        )
        .await
    }
    async fn channel(&self, request: Request) -> vz_mcp::channel::Stream {
        #[cfg(unix)]
        let (mut client, server) = tokio::net::UnixStream::pair().unwrap();
        #[cfg(windows)]
        let (mut client, server) = {
            let server=vz_mcp::channel::create_server(false).unwrap();
            let client=vz_mcp::channel::connect().await.unwrap();
            server.connect().await.unwrap();
            (client,server)
        };
        let (dir, root, ops, apps, opens) = (
            self.dir.clone(),
            self.root.clone(),
            self.ops.clone(),
            self.apps.clone(),
            self.opens.clone(),
        );
        tokio::spawn(mcp_channel::serve(
            server,
            dir.clone(),
            move |r, c| async move {
                mcp_tools::call(&dir, &root, &ops, &apps, r, c, move |_, _| {
                    opens.fetch_add(1, Ordering::SeqCst);
                    Ok(json!({"hits":[{"text":"private"}]}))
                })
                .await
            },
        ));
        let bytes = serde_json::to_vec(
            &json!({"v":1,"client":request.client,"tool":request.tool,"args":request.args}),
        )
        .unwrap();
        vz_mcp::channel::write_frame(&mut client, &bytes, vz_mcp::REQUEST_CAP)
            .await
            .unwrap();
        client
    }
}
#[tokio::test]
async fn every_tool_denied_without_index_opens() {
    for tool in ["list_collections", "search", "get_document", "get_chunk"] {
        for case in 0..9 {
            let f = Fixture::new().await;
            settings::with_state(&f.dir, |s| match case {
                0 => s.settings.0["paths"]["app"]["enabled"] = json!(false),
                1 => s.app_off = true,
                2 => s.settings.0["paths"]["app"]["clients"][0]["revoked_at"] = json!("today"),
                3 => {
                    s.app_revoked.insert(f.id.clone());
                }
                4 => {
                    s.settings.0["paths"]["app"]["clients"][0]["consent"]["disclosure_version"] =
                        json!("stale")
                }
                5 => s.settings.0["paths"]["app"]["clients"][0]["grants"] = json!([]),
                _ => {}
            })
            .unwrap();
            let mut r = f.request(tool);
            if case == 6 {
                r.client = "f".repeat(32);
            }
            if case == 7 {
                r.client = "bad".into();
            }
            if case == 8 {
                r.v = 0;
            }
            assert_eq!(f.call(r).await["ok"], false, "{tool} case {case}");
            assert_eq!(f.opens.load(Ordering::SeqCst), 0);
        }
    }
}
#[tokio::test]
async fn off_retry_on_and_per_client_failed_denials() {
    let f = Fixture::new().await;
    assert_eq!(f.call(f.request("search")).await["ok"], true);
    f.apps.path_set(&f.dir, false).await.unwrap();
    assert_eq!(f.call(f.request("search")).await["ok"], false);
    f.apps.path_set(&f.dir, true).await.unwrap();
    assert_eq!(f.call(f.request("search")).await["ok"], true);
    f.fail_writes();
    let off = f.apps.path_set(&f.dir, false).await.unwrap();
    assert_eq!(off["saved"], false);
    assert!(f.apps.path_set(&f.dir, true).await.is_err());
    assert_eq!(f.call(f.request("search")).await["ok"], false);
    f.fix_writes();
    f.apps.retry(&f.dir).await.unwrap();
    f.apps.path_set(&f.dir, true).await.unwrap();
    assert_eq!(f.call(f.request("search")).await["ok"], true);
    f.fail_writes();
    let disconnected = f.apps.disconnect(&f.dir, &f.id).await.unwrap();
    assert_eq!(disconnected["saved"], false);
    let second = f
        .apps
        .connect(
            &f.dir,
            "other",
            "Same name",
            &vz_core::cloud_gate::app_disclosure_version(),
        )
        .await;
    assert!(second.is_err());
    assert_eq!(f.call(f.request("search")).await["ok"], false);
    f.fix_writes();
    f.apps.retry(&f.dir).await.unwrap();
    settings::restart(&f.dir);
    assert_eq!(f.call(f.request("search")).await["ok"], false);
}
#[tokio::test]
async fn off_and_on_serialize_through_failed_or_successful_persistence() {
    for fail in [false, true] {
        let f = Fixture::new().await;
        if fail {
            f.fail_writes();
        }
        let (drained, resume) = mcp_channel::hook(&f.dir, "off_drained");
        let (apps, dir) = (f.apps.clone(), f.dir.clone());
        let off = tokio::spawn(async move { apps.path_set(&dir, false).await });
        drained.await.unwrap();
        let (apps, dir) = (f.apps.clone(), f.dir.clone());
        let on = tokio::spawn(async move { apps.path_set(&dir, true).await });
        tokio::task::yield_now().await;
        assert!(!on.is_finished());
        assert_eq!(f.call(f.request("search")).await["ok"], false);
        assert!(settings::with_state(&f.dir, |s| s.app_deliveries.is_empty()).unwrap());
        resume.send(()).unwrap();
        off.await.unwrap().unwrap();
        if fail {
            assert!(on.await.unwrap().is_err());
            f.fix_writes();
            f.apps.path_set(&f.dir, true).await.unwrap();
        } else {
            on.await.unwrap().unwrap();
        }
        assert_eq!(f.call(f.request("search")).await["ok"], true);
    }
}
#[tokio::test]
async fn confirm_keeps_id_and_cannot_reactivate_revoked_client() {
    let f = Fixture::new().await;
    let version = vz_core::cloud_gate::app_disclosure_version();
    settings::update(&f.dir, |s| {
        s.settings.0["paths"]["app"]["clients"][0]["consent"]["disclosure_version"] = json!("old");
        Ok::<_, String>(())
    })
    .unwrap();
    assert!(f.apps.confirm(&f.dir, &f.id, "old").await.is_err());
    f.apps.confirm(&f.dir, &f.id, &version).await.unwrap();
    assert_eq!(f.call(f.request("search")).await["ok"], true);
    f.apps.disconnect(&f.dir, &f.id).await.unwrap();
    assert!(f.apps.confirm(&f.dir, &f.id, &version).await.is_err());
}
#[tokio::test]
async fn cancelled_before_write_sends_zero_bytes_and_blocked_write_drains_under_one_second() {
    use tokio::io::AsyncReadExt;
    let token = CancellationToken::new();
    token.cancel();
    let (mut writer, mut reader) = tokio::io::duplex(32);
    assert!(
        mcp_channel::deliver(&mut writer, &json!({"private":"content"}), &token)
            .await
            .is_err()
    );
    drop(writer);
    let mut bytes = vec![];
    reader.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
    for fail in [false, true] {
        let f = Fixture::new().await;
        if fail {
            f.fail_writes();
        }
        let delivery = Delivery::register(&f.dir, &f.request("search")).unwrap();
        let token = delivery.token.clone();
        let (mut writer, _reader) = tokio::io::duplex(32);
        let write = tokio::spawn(async move {
            let _guard = delivery;
            mcp_channel::deliver(&mut writer, &json!({"private":"x".repeat(800000)}), &token).await
        });
        tokio::task::yield_now().await;
        tokio::time::timeout(Duration::from_secs(1), f.apps.disconnect(&f.dir, &f.id))
            .await
            .unwrap()
            .unwrap();
        assert!(write.await.unwrap().is_err());
        assert!(settings::with_state(&f.dir, |s| s.app_deliveries.is_empty()).unwrap());
        assert_eq!(f.call(f.request("search")).await["ok"], false);
    }
}
#[tokio::test]
async fn real_channel_gate3_pause_and_handoff_with_failed_persistence() {
    for point in ["after_tool", "gate3"] {
        for fail in [false, true] {
            let f = Fixture::new().await;
            if fail {
                f.fail_writes();
            }
            let (reached, resume) = mcp_channel::hook(&f.dir, point);
            let mut client = f.channel(f.request("search")).await;
            reached.await.unwrap();
            let (apps, dir, id) = (f.apps.clone(), f.dir.clone(), f.id.clone());
            let revoke = tokio::spawn(async move { apps.disconnect(&dir, &id).await });
            if point == "gate3" {
                let token = settings::with_state(&f.dir, |s| {
                    s.app_deliveries.values().next().unwrap().1.clone()
                })
                .unwrap();
                token.cancelled().await;
                assert!(!revoke.is_finished());
            } else {
                revoke.await.unwrap().unwrap();
                resume.send(()).unwrap();
                let bytes = vz_mcp::channel::read_frame(&mut client, vz_mcp::RESPONSE_CAP)
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::from_slice::<Value>(&bytes).unwrap()["ok"],
                    false
                );
                continue;
            }
            resume.send(()).unwrap();
            revoke.await.unwrap().unwrap();
            assert!(
                vz_mcp::channel::read_frame(&mut client, vz_mcp::RESPONSE_CAP)
                    .await
                    .is_err()
            );
        }
    }
    let f = Fixture::new().await;
    let mut client = f.channel(f.request("search")).await;
    let handed = vz_mcp::channel::read_frame(&mut client, vz_mcp::RESPONSE_CAP)
        .await
        .unwrap();
    f.apps.disconnect(&f.dir, &f.id).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&handed).unwrap()["ok"],
        true
    );
    let mut next = f.channel(f.request("search")).await;
    let bytes = vz_mcp::channel::read_frame(&mut next, vz_mcp::RESPONSE_CAP)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["ok"],
        false
    );
}
#[tokio::test]
async fn revoke_while_waiting_for_inference_never_opens_index() {
    let f = Fixture::new().await;
    let held = f.ops.acquire().unwrap();
    let (dir, root, ops, apps, opens) = (
        f.dir.clone(),
        f.root.clone(),
        f.ops.clone(),
        f.apps.clone(),
        f.opens.clone(),
    );
    let request = f.request("search");
    let notify = settings::with_state(&f.dir, |s| s.app_notify.clone()).unwrap();
    let notified = notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    let call = tokio::spawn(async move {
        mcp_tools::call(
            &dir,
            &root,
            &ops,
            &apps,
            request,
            CancellationToken::new(),
            move |_, _| {
                opens.fetch_add(1, Ordering::SeqCst);
                Ok(json!({}))
            },
        )
        .await
    });
    notified.await;
    f.apps.disconnect(&f.dir, &f.id).await.unwrap();
    drop(held);
    assert_eq!(call.await.unwrap()["ok"], false);
    assert_eq!(f.opens.load(Ordering::SeqCst), 0);
}
