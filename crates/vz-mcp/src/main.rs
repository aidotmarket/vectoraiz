use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use vz_mcp::protocol;

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let client = args
        .windows(2)
        .find(|w| w[0] == "--client")
        .map(|w| w[1].clone())
        .unwrap_or_default();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut requests: HashMap<String, CancellationToken> = HashMap::new();
    let mut initialized = false;
    let mut tasks = tokio::task::JoinSet::new();
    while let Ok(Some(line)) = lines.next_line().await {
        requests.retain(|_, t| !t.is_cancelled());
        let request: Value = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => {
                let _ = send(
                    &stdout,
                    &protocol::error(Value::Null, -32700, "Parse error"),
                    &CancellationToken::new(),
                )
                .await;
                continue;
            }
        };
        if request.get("id").is_none() {
            match request["method"].as_str() {
                Some("notifications/cancelled") => {
                    if let Some(token) =
                        requests.remove(&request["params"]["requestId"].to_string())
                    {
                        token.cancel();
                    }
                }
                Some("notifications/initialized") => initialized = true,
                _ => {}
            }
            continue;
        }
        let (modern, immediate) = protocol::immediate(&request, initialized);
        if let Some(response) = immediate {
            let _ = send(&stdout, &response, &CancellationToken::new()).await;
            continue;
        }
        let key = request["id"].to_string();
        if requests.get(&key).is_some_and(|t| !t.is_cancelled()) {
            let _ = send(
                &stdout,
                &protocol::error(request["id"].clone(), -32600, "Duplicate request id"),
                &CancellationToken::new(),
            )
            .await;
            continue;
        }
        let token = CancellationToken::new();
        requests.insert(key, token.clone());
        let (stdout, client) = (stdout.clone(), client.clone());
        tasks.spawn(async move {
            let work=async {
                let tool_result=if !vz_mcp::valid_client(&client) {protocol::tool_error(vz_mcp::NO_ACCESS)} else {
                    let bytes=serde_json::to_vec(&json!({"v":1,"client":client,"tool":request["params"]["name"],"args":request["params"]["arguments"]})).unwrap();
                    match vz_mcp::channel::call(&bytes).await {
                        Ok(bytes)=>match serde_json::from_slice::<Value>(&bytes) {
                            Ok(v) if v["ok"]==true=> {let object=v["result"].clone();json!({"content":[{"type":"text","text":object.to_string()}],"structuredContent":object,"isError":false})},
                            Ok(v)=>protocol::tool_error(v["message"].as_str().unwrap_or(vz_mcp::NO_ACCESS)),
                            Err(_)=>protocol::tool_error(vz_mcp::NO_ACCESS),
                        },
                        Err(e)=>protocol::tool_error(if matches!(e.kind(),std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset) {vz_mcp::NO_ACCESS}else{vz_mcp::NOT_RUNNING}),
                    }
                };
                let _=send(&stdout,&protocol::result(request["id"].clone(),tool_result,modern),&token).await;
            };
            tokio::select! {biased;_ = token.cancelled()=>{},_ = work=>{}}
            token.cancel();
        });
        while tasks.try_join_next().is_some() {}
    }
    for token in requests.values() {
        token.cancel();
    }
    while tasks.join_next().await.is_some() {}
}
async fn send<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &Arc<Mutex<W>>,
    value: &Value,
    token: &CancellationToken,
) -> std::io::Result<()> {
    let mut stdout = stdout.lock().await;
    if token.is_cancelled() {
        return Ok(());
    }
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    stdout.write_all(&bytes).await?;
    stdout.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handed_off_result_paused_before_stdout_can_arrive_after_removal() {
        // The stdout mutex is a deterministic hook before any stdout byte is written.
        // The app-side real-channel test proves which frames can cross handoff.
        let (writer, reader) = tokio::io::duplex(4096);
        let stdout = Arc::new(Mutex::new(writer));
        let pause = stdout.lock().await;
        let (handed_off, received) = tokio::sync::oneshot::channel();
        let output = stdout.clone();
        let task = tokio::spawn(async move {
            let frame = json!({"ok":true,"result":{"hits":[{"text":"already handed off"}]}});
            // Decode the same complete local-channel frame the bridge relays.
            let mut wire = vec![];
            vz_mcp::channel::write_frame(
                &mut wire,
                &serde_json::to_vec(&frame).unwrap(),
                vz_mcp::RESPONSE_CAP,
            )
            .await
            .unwrap();
            let bytes = vz_mcp::channel::read_frame(&mut wire.as_slice(), vz_mcp::RESPONSE_CAP)
                .await
                .unwrap();
            let response: Value = serde_json::from_slice(&bytes).unwrap();
            handed_off.send(()).unwrap();
            send(
                &output,
                &protocol::result(json!(1), response["result"].clone(), true),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        });
        received.await.unwrap();
        assert!(!task.is_finished());
        // Removal returns while the bridge is paused. It cannot recall this frame.
        drop(pause);
        task.await.unwrap();
        let mut lines = BufReader::new(reader).lines();
        let response: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(response["result"]["hits"][0]["text"], "already handed off");
        let token = CancellationToken::new();
        token.cancel();
        send(&stdout, &json!({"text":"cancelled new result"}), &token)
            .await
            .unwrap();
        drop(stdout);
        assert!(lines.next_line().await.unwrap().is_none());
    }
}
