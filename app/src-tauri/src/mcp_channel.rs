//! One request per authenticated local connection. Revocation owns the delivery boundary.
use crate::settings;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use vz_core::cloud_gate::{check_mcp, GateError};

#[derive(Default)]
pub struct ChannelState {
    pub started: std::sync::atomic::AtomicBool,
    pub error: Mutex<Option<String>>,
    pub stop: CancellationToken,
}
#[derive(Deserialize)]
pub struct Request {
    pub v: u32,
    pub client: String,
    pub tool: String,
    pub args: Value,
}
pub fn refusal(error: GateError) -> Value {
    let (code, message) = if error == GateError::ConsentRequired {
        (
            "consent_required",
            "Open vectorAIz to confirm the updated terms.",
        )
    } else {
        ("access_denied", vz_mcp::NO_ACCESS)
    };
    json!({"ok":false,"code":code,"message":message})
}
pub fn gate(dir: &Path, request: &Request) -> Result<(), Value> {
    if request.v != 1 || !vz_mcp::valid_client(&request.client) {
        return Err(refusal(GateError::Revoked));
    }
    settings::with_state(dir, |s| {
        check_mcp(&s.app_state(), &request.client, &request.tool, "library")
    })
    .map_err(|_| refusal(GateError::Revoked))?
    .map_err(refusal)
}

pub struct Delivery {
    dir: PathBuf,
    id: String,
    pub token: CancellationToken,
}

#[cfg(test)]
type Hook = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);
#[cfg(test)]
fn hooks() -> &'static Mutex<std::collections::HashMap<(PathBuf, &'static str), Hook>> {
    static HOOKS: std::sync::OnceLock<
        Mutex<std::collections::HashMap<(PathBuf, &'static str), Hook>>,
    > = std::sync::OnceLock::new();
    HOOKS.get_or_init(Default::default)
}
#[cfg(test)]
pub fn hook(
    dir: &Path,
    point: &'static str,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (reached, receiver) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    hooks()
        .lock()
        .unwrap()
        .insert((dir.to_owned(), point), (reached, resume));
    (receiver, release)
}
#[cfg(test)]
pub async fn pause(dir: &Path, point: &'static str) {
    let hook = hooks().lock().unwrap().remove(&(dir.to_owned(), point));
    if let Some((reached, resume)) = hook {
        let _ = reached.send(());
        let _ = resume.await;
    }
}
impl Delivery {
    pub fn register(dir: &Path, request: &Request) -> Result<Self, Value> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let token = CancellationToken::new();
        settings::with_state(dir, |s| {
            check_mcp(&s.app_state(), &request.client, &request.tool, "library")
                .map_err(refusal)?;
            s.app_deliveries
                .insert(id.clone(), (request.client.clone(), token.clone()));
            Ok(Self {
                dir: dir.to_owned(),
                id,
                token,
            })
        })
        .map_err(|_| refusal(GateError::Revoked))?
    }
}
impl Drop for Delivery {
    fn drop(&mut self) {
        let _ = settings::with_state(&self.dir, |s| {
            s.app_deliveries.remove(&self.id);
            s.app_notify.notify_waiters();
        });
    }
}
pub async fn deliver<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
    token: &CancellationToken,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > vz_mcp::RESPONSE_CAP {
        return Err(io::Error::other("response exceeds limit"));
    }
    let frame = [&(bytes.len() as u32).to_le_bytes()[..], &bytes].concat();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        for chunk in frame.chunks(4096) {
            let mut remaining = chunk;
            while !remaining.is_empty() {
                let n = tokio::select! { biased;
                    _ = token.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted, "delivery cancelled")),
                    n = writer.write(remaining) => n?,
                };
                if n == 0 { return Err(io::Error::from(io::ErrorKind::WriteZero)); }
                remaining = &remaining[n..];
            }
        }
        Ok(())
    }).await.unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::TimedOut)));
    // serve owns both halves and drops them on this return. Never await a
    // peer-dependent shutdown after the write deadline or cancellation.
    result
}

pub async fn serve<S, F, Fut>(stream: S, dir: PathBuf, run: F)
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(Request, CancellationToken) -> Fut,
    Fut: std::future::Future<Output = Value>,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let bytes = match tokio::time::timeout(
        Duration::from_secs(30),
        vz_mcp::channel::read_frame(&mut reader, vz_mcp::REQUEST_CAP),
    )
    .await
    {
        Ok(Ok(b)) => b,
        _ => return,
    };
    let request: Request = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return,
    };
    let release = Request {
        v: request.v,
        client: request.client.clone(),
        tool: request.tool.clone(),
        args: Value::Null,
    };
    let cancel = CancellationToken::new();
    let mut byte = [0u8];
    let work = async {
        let value = run(request, cancel.clone()).await;
        if value["ok"] == true {
            match Delivery::register(&dir, &release) {
                Ok(delivery) => {
                    #[cfg(test)]
                    pause(&dir, "gate3").await;
                    tokio::select! { biased;
                        _ = cancel.cancelled() => delivery.token.cancel(),
                        _ = deliver(&mut writer, &value, &delivery.token) => {},
                    }
                }
                Err(error) => {
                    let _ = deliver(&mut writer, &error, &cancel).await;
                }
            }
        } else {
            let _ = deliver(&mut writer, &value, &cancel).await;
        }
    };
    tokio::select! { biased;
        _ = reader.read(&mut byte) => { cancel.cancel(); },
        _ = work => {},
    }
}

// Per-connection failures do not change the listener's startup status.
fn connection_result<T>(result: io::Result<T>) -> Option<T> {
    match result {
        Ok(connection) => Some(connection),
        Err(error) => {
            eprintln!("local channel connection failed: {error}");
            None
        }
    }
}

#[cfg(any(unix, test))]
async fn accept_next<T, F, Fut>(stop: &CancellationToken, mut accept: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = io::Result<T>>,
{
    loop {
        let result = tokio::select! { biased;
            _ = stop.cancelled() => return None,
            result = accept() => result,
        };
        if let Some(connection) = connection_result(result) {
            return Some(connection);
        }
        tokio::select! {
            _ = stop.cancelled() => return None,
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

pub async fn listen<F, Fut>(state: Arc<ChannelState>, dir: PathBuf, run: F)
where
    F: Fn(Request, CancellationToken) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Value> + Send + 'static,
{
    let run = Arc::new(run);
    #[cfg(unix)]
    let result = async {
        let listener = vz_mcp::channel::bind().await?;
        loop {
            let Some((stream, _)) = accept_next(&state.stop, || listener.accept()).await else {
                break;
            };
            if vz_mcp::channel::verify_peer(&stream).is_err() {
                continue;
            }
            let (dir, run) = (dir.clone(), run.clone());
            tokio::spawn(async move {
                serve(stream, dir, move |r, c| run(r, c)).await;
            });
        }
        Ok::<_, io::Error>(())
    }
    .await;
    #[cfg(windows)]
    let result = async {
        let mut server = vz_mcp::channel::create_server(true)?;
        loop {
            tokio::select! { biased;
                _ = state.stop.cancelled() => break,
                result = server.connect() => if connection_result(result).is_none() {
                    tokio::select! {
                        _ = state.stop.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    }
                    continue;
                },
            };
            let next = loop {
                if let Some(next) = connection_result(vz_mcp::channel::create_server(false)) {
                    break next;
                }
                tokio::select! {
                    _ = state.stop.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
            };
            let connected = std::mem::replace(&mut server, next);
            let (dir, run) = (dir.clone(), run.clone());
            tokio::spawn(async move {
                serve(connected, dir, move |r, c| run(r, c)).await;
            });
        }
        Ok::<_, io::Error>(())
    }
    .await;
    if result.is_err() {
        eprintln!("AI apps local channel unavailable");
        *state.error.lock().unwrap() = Some(
            "vectorAIz can't open its local channel. Restart your computer, then try again.".into(),
        );
    }
}

#[cfg(test)]
mod accept_tests {
    use super::*;
    #[tokio::test]
    async fn failing_accept_then_next_connection_is_served() {
        let state = ChannelState::default();
        let (client, server) = tokio::io::duplex(4096);
        let mut accepts = vec![
            Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
            Ok(server),
        ]
        .into_iter();
        let stream = accept_next(&state.stop, || std::future::ready(accepts.next().unwrap()))
            .await
            .unwrap();
        let writer = tokio::spawn(async move {
            let mut client = client;
            vz_mcp::channel::write_frame(
                &mut client,
                &serde_json::to_vec(
                    &json!({"v":1,"client":"a".repeat(32),"tool":"list_collections","args":{}}),
                )
                .unwrap(),
                vz_mcp::REQUEST_CAP,
            )
            .await
            .unwrap();
            let bytes = vz_mcp::channel::read_frame(&mut client, vz_mcp::RESPONSE_CAP)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap()["message"],
                "test refusal"
            );
        });
        serve(stream, PathBuf::new(), |_, _| async {
            json!({"ok":false,"message":"test refusal"})
        })
        .await;
        writer.await.unwrap();
        assert!(accepts.next().is_none());
        assert!(state.error.lock().unwrap().is_none());
    }
}
