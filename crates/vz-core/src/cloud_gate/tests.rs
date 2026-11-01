use super::*;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
};
fn state(p: Provider) -> ConsentState {
    ConsentState {
        enabled: true,
        provider: Some(p),
        consent: Some(Consent {
            provider: p,
            disclosure_version: disclosure_version(),
            confirmed_at: "2026-11-01T00:00:00Z".into(),
        }),
        key_present: true,
        stopped: false,
    }
}
#[test]
fn consent_state_table_both_ops() {
    for p in [Provider::Anthropic, Provider::OpenAi] {
        let other = if p == Provider::Anthropic {
            Provider::OpenAi
        } else {
            Provider::Anthropic
        };
        let good = state(p);
        for (s, ask, key) in [
            (
                ConsentState::default(),
                Err(GateError::PathOff),
                Err(GateError::ConsentRequired),
            ),
            (good.clone(), Ok(()), Ok(())),
            (
                ConsentState {
                    key_present: false,
                    ..good.clone()
                },
                Err(GateError::NoKey),
                Ok(()),
            ),
            (
                ConsentState {
                    enabled: false,
                    ..good.clone()
                },
                Err(GateError::PathOff),
                Ok(()),
            ),
            (
                ConsentState {
                    consent: Some(Consent {
                        provider: p,
                        disclosure_version: "stale".into(),
                        confirmed_at: String::new(),
                    }),
                    ..good.clone()
                },
                Err(GateError::ConsentRequired),
                Err(GateError::ConsentRequired),
            ),
            (
                ConsentState {
                    consent: state(other).consent,
                    ..good.clone()
                },
                Err(GateError::ConsentRequired),
                Err(GateError::ConsentRequired),
            ),
            (
                ConsentState {
                    stopped: true,
                    ..good.clone()
                },
                Err(GateError::Revoked),
                Err(GateError::Revoked),
            ),
            (
                ConsentState {
                    provider: Some(other),
                    ..good.clone()
                },
                Err(GateError::ConsentRequired),
                Ok(()),
            ),
        ] {
            assert_eq!(check(&s, p, Op::Ask), ask);
            assert_eq!(check(&s, p, Op::KeyCheck), key);
        }
    }
}
#[test]
fn allow_list_binds_provider_and_https() {
    for p in [Provider::Anthropic, Provider::OpenAi] {
        assert!(allowed(
            p,
            &Url::parse(&format!("https://{}/v1/models", p.host())).unwrap()
        ));
        for host in ["ai.market", "api.ai.market", "example.com", "localhost"] {
            assert!(!allowed(
                p,
                &Url::parse(&format!("https://{host}/")).unwrap()
            ));
        }
        assert!(!allowed(
            p,
            &Url::parse(&format!("http://{}/", p.host())).unwrap()
        ));
        let other = if p == Provider::Anthropic {
            Provider::OpenAi
        } else {
            Provider::Anthropic
        };
        assert!(!allowed(
            p,
            &Url::parse(&format!("https://{}/", other.host())).unwrap()
        ));
    }
}
#[test]
fn request_contract_unicode_limit_and_caps() {
    for p in [Provider::Anthropic, Provider::OpenAi] {
        let body = request(p, &"界".repeat(2000), "[1] passage").unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(
            request(p, &"α".repeat(2001), "").unwrap_err(),
            SendError::QuestionTooLong
        );
        match p {
            Provider::Anthropic => {
                assert_eq!(body["max_tokens"], 4096);
                assert_eq!(body["thinking"]["type"], "between_tools");
                assert_eq!(body["output_config"]["effort"], "low");
            }
            Provider::OpenAi => {
                assert_eq!(body["max_output_tokens"], 16000);
                assert_eq!(body["reasoning"]["effort"], "low");
                assert_eq!(body["store"], false);
            }
        }
    }
}
fn sse(v: Value) -> String {
    format!("data: {v}\n\n")
}
fn transcript(p: Provider) -> String {
    match p {
 Provider::Anthropic=>[json!({"type":"ping"}),json!({"type":"content_block_start","content_block":{"type":"thinking"}}),json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hidden"}}),json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"κόσμος [1]"}}),json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),json!({"type":"message_stop"})].into_iter().map(sse).collect(),
 Provider::OpenAi=>[json!({"type":"response.output_item.added","item":{"type":"reasoning"}}),json!({"type":"response.output_text.delta","delta":"κόσμος [1]"}),json!({"type":"response.completed"})].into_iter().map(sse).collect(),
 }
}
#[test]
fn framing_split_every_byte_and_multiframe_reads() {
    for p in [Provider::Anthropic, Provider::OpenAi] {
        for width in [1, 2, 7, 65536] {
            let text = transcript(p).replace('\n', "\r\n");
            let mut parser = Parser::default();
            let mut emitted = String::new();
            for chunk in text.as_bytes().chunks(width) {
                parser.push(chunk, p, &mut |t| emitted.push_str(t)).unwrap();
            }
            assert!(parser.terminal);
            assert_eq!(parser.text, "κόσμος [1]");
            assert_eq!(emitted, parser.text);
        }
    }
}
#[test]
fn stream_failures_discard_partial_answers() {
    for (reason, expected) in [
        ("refusal", SendError::Declined),
        ("max_tokens", SendError::Incomplete),
        ("unexpected", SendError::ProviderError),
        ("model_context_window_exceeded", SendError::RequestRejected),
    ] {
        let input = sse(json!({"type":"message_delta","delta":{"stop_reason":reason}}))
            + &sse(json!({"type":"message_stop"}));
        assert_eq!(
            Parser::default().push(input.as_bytes(), Provider::Anthropic, &mut |_| {}),
            Err(expected)
        );
    }
    for (event, expected) in [
        (json!({"type":"response.incomplete"}), SendError::Incomplete),
        (
            json!({"type":"error","error":{"type":"rate_limit_error"}}),
            SendError::RateLimited,
        ),
        (
            json!({"type":"error","error":{"type":"api_error"}}),
            SendError::ProviderError,
        ),
    ] {
        assert_eq!(
            Parser::default().push(sse(event).as_bytes(), Provider::OpenAi, &mut |_| {}),
            Err(expected)
        );
    }
    let mut parser = Parser::default();
    parser
        .push(
            sse(json!({"type":"response.refusal.done","refusal":"no"})).as_bytes(),
            Provider::OpenAi,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(
        parser.push(
            sse(json!({"type":"response.completed"})).as_bytes(),
            Provider::OpenAi,
            &mut |_| {}
        ),
        Err(SendError::Declined)
    );
    assert!(!Parser::default().terminal);
}
#[test]
fn failure_mapping_table() {
    use SendError::*;
    for p in [Provider::Anthropic, Provider::OpenAi] {
        for (status, expected) in [
            (401, KeyRejected),
            (403, KeyRejected),
            (404, ModelUnavailable),
            (400, RequestRejected),
            (413, RequestRejected),
            (302, RequestRejected),
            (500, ProviderError),
            (504, ProviderError),
            (529, ProviderError),
            (418, ProviderError),
        ] {
            assert_eq!(map_failure(p, status, true, &Value::Null), expected);
        }
        assert_eq!(
            map_failure(p, 429, true, &json!({"error":{"type":"rate_limit_error"}})),
            RateLimited
        );
    }
    for (status, retry, value) in [
        (402, true, json!({})),
        (429, false, json!({"error":{"type":"rate_limit_error"}})),
        (
            429,
            true,
            json!({"error":{"details":{"error_code":"enforced_spend_limit_reached"}}}),
        ),
        (
            400,
            true,
            json!({"error":{"type":"invalid_request_error","message":"You have reached your specified API usage limits"}}),
        ),
    ] {
        assert_eq!(
            map_failure(Provider::Anthropic, status, retry, &value),
            Quota
        );
    }
    for code in [
        "insufficient_quota",
        "credit_balance_exhausted",
        "organization_spend_limit_exceeded",
        "project_spend_limit_exceeded",
        "organization_usage_limit_exceeded",
    ] {
        assert_eq!(
            map_failure(Provider::OpenAi, 429, true, &json!({"error":{"code":code}})),
            Quota
        );
    }
}
struct Server {
    child: Child,
    root: tempfile::TempDir,
    client: ProviderClient,
    addr: std::net::SocketAddr,
}
impl Server {
    fn new(plan: Value) -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("plan.json"), plan.to_string()).unwrap();
        let mut child = Command::new(if cfg!(windows) { "python" } else { "python3" })
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/models-test-server.py"))
            .arg(root.path())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        assert!(!port.trim().is_empty());
        let cert = reqwest::Certificate::from_pem(&fs::read(root.path().join("cert.pem")).unwrap())
            .unwrap();
        let addr = format!("127.0.0.1:{}", port.trim()).parse().unwrap();
        let client = if plan[0]["short_timeout"] == true {
            ProviderClient::timed_for_test(addr, cert)
        } else {
            ProviderClient::for_test(addr, cert)
        };
        Self {
            child,
            root,
            client,
            addr,
        }
    }
    fn count(&self) -> usize {
        fs::read(self.root.path().join("requests.json"))
            .ok()
            .map(|b| serde_json::from_slice::<Vec<Value>>(&b).unwrap().len())
            .unwrap_or(0)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[tokio::test]
async fn real_tls_transport_zero_requests_and_one_check_then_ask() {
    for p in [Provider::Anthropic, Provider::OpenAi] {
        let server = Server::new(json!([{"body":"{}"},{"body":transcript(p)}]));
        let (_tx, mut rx) = watch::channel(false);
        let key = Secret::new("sk-test-SENTINEL-not-a-real-key".into());
        for (state, epoch) in [
            (ConsentState::default(), 0),
            (
                ConsentState {
                    stopped: true,
                    ..state(p)
                },
                0,
            ),
            (
                ConsentState {
                    enabled: false,
                    ..state(p)
                },
                0,
            ),
            (
                ConsentState {
                    consent: None,
                    ..state(p)
                },
                0,
            ),
            (state(p), 1),
        ] {
            assert!(dispatch(
                &server.client,
                || (state.clone(), epoch),
                p,
                Op::Ask,
                &key,
                None,
                &0,
                &mut rx
            )
            .await
            .is_err());
        }
        assert_eq!(server.count(), 0);
        dispatch(
            &server.client,
            || (state(p), 0),
            p,
            Op::KeyCheck,
            &key,
            None,
            &0,
            &mut rx,
        )
        .await
        .unwrap();
        let stream = dispatch(
            &server.client,
            || (state(p), 0),
            p,
            Op::Ask,
            &key,
            Some(request(p, "question", "[1] passage").unwrap()),
            &0,
            &mut rx,
        )
        .await
        .unwrap();
        let answer = stream
            .answer(
                || true,
                &mut rx,
                |_| {},
                vec![Citation {
                    number: 1,
                    doc_id: "doc".into(),
                    page: 1,
                    chunk_id: "chunk".into(),
                }],
            )
            .await
            .unwrap();
        assert!(answer.text.contains("κόσμος [1]"));
        assert_eq!(answer.citations.len(), 1);
        assert_eq!(server.count(), 2);
        let log = fs::read_to_string(server.root.path().join("requests.json")).unwrap();
        assert!(!log.contains("SENTINEL"));
        let requests: Vec<Value> = serde_json::from_str(&log).unwrap();
        assert_eq!(requests[0]["method"], "GET");
        assert_eq!(requests[0]["path"], "/v1/models");
        assert_eq!(requests[1]["host"], p.host());
    }
}
#[tokio::test]
async fn tls_redirect_http_errors_and_stall_cancellation() {
    let p = Provider::Anthropic;
    let key = Secret::new("sk-test-only".into());
    for (action, expected) in [
        (
            json!({"status":302,"headers":{"Location":"https://example.com/"},"body":""}),
            SendError::RequestRejected,
        ),
        (
            json!({"status":401,"body":"private body"}),
            SendError::KeyRejected,
        ),
        (json!({"status":429,"body":"{}"}), SendError::Quota),
    ] {
        let server = Server::new(json!([action]));
        let (_tx, mut rx) = watch::channel(false);
        let result = dispatch(
            &server.client,
            || (state(p), 0),
            p,
            Op::KeyCheck,
            &key,
            None,
            &0,
            &mut rx,
        )
        .await;
        assert!(matches!(result,Err(e) if e==expected));
        assert_eq!(server.count(), 1);
    }
    let server = Server::new(json!([{"body":sse(json!({"type":"ping"})),"body_delay":5}]));
    let (tx, mut rx) = watch::channel(false);
    let stream = dispatch(
        &server.client,
        || (state(p), 0),
        p,
        Op::Ask,
        &key,
        Some(json!({})),
        &0,
        &mut rx,
    )
    .await
    .unwrap();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = tx.send(true);
    });
    assert_eq!(
        stream
            .answer(|| true, &mut rx, |_| {}, vec![])
            .await
            .unwrap_err(),
        SendError::Cancelled
    );
}

#[test]
fn passages_are_trimmed_whole_with_contiguous_citations() {
    let hits = (0..3)
        .map(|i| crate::storage::Hit {
            score: 1.0,
            chunk: crate::pipeline::Chunk {
                chunk_id: i.to_string(),
                doc_id: format!("private-file-{i}"),
                text: "界".repeat(3000),
                page: 42,
                heading_path: vec![],
                token_count: 3000,
                embedding: vec![],
            },
        })
        .collect::<Vec<_>>();
    let (context, citations) = crate::index::cloud_passages(&hits);
    assert!(context.len() <= 24000);
    assert_eq!(citations.len(), 2);
    assert_eq!(
        citations.iter().map(|c| c.number).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(!context.contains("private-file"));
    assert!(!context.contains("42"));
    assert!(!context.contains("[3]"));
}
#[tokio::test]
async fn malformed_eof_untrusted_tls_and_read_timeout_are_typed() {
    let p = Provider::OpenAi;
    let key = Secret::new("sk-test-only".into());
    let server = Server::new(
        json!([{"body":sse(json!({"type":"response.output_text.delta","delta":"partial"}))}]),
    );
    let (_tx, mut rx) = watch::channel(false);
    let stream = dispatch(
        &server.client,
        || (state(p), 0),
        p,
        Op::Ask,
        &key,
        Some(json!({})),
        &0,
        &mut rx,
    )
    .await
    .unwrap();
    assert_eq!(
        stream
            .answer(|| true, &mut rx, |_| {}, vec![])
            .await
            .unwrap_err(),
        SendError::ProviderError
    );
    let server = Server::new(json!([{"body":"{}"}]));
    let untrusted = ProviderClient::untrusted_for_test(server.addr);
    let failed = dispatch(
        &untrusted,
        || (state(p), 0),
        p,
        Op::KeyCheck,
        &key,
        None,
        &0,
        &mut rx,
    )
    .await;
    assert!(matches!(failed, Err(SendError::Unreachable)));
    assert_eq!(server.count(), 0);
    let address = server
        .client
        .0
        .get(format!("http://{}/", p.host()))
        .send()
        .await;
    assert!(address.is_err());
    assert_eq!(server.count(), 0);
    let server = Server::new(json!([{"body":"data: {}\n\n","body_delay":5,"short_timeout":true}]));
    let stream = dispatch(
        &server.client,
        || (state(p), 0),
        p,
        Op::Ask,
        &key,
        Some(json!({})),
        &0,
        &mut rx,
    )
    .await
    .unwrap();
    assert_eq!(
        stream
            .answer(|| true, &mut rx, |_| {}, vec![])
            .await
            .unwrap_err(),
        SendError::ProviderError
    );
}

#[test]
fn multiline_data_and_unknown_frames_are_ignored() {
    let mut parser = Parser::default();
    parser.push(b"event: ping\ndata: keepalive\n\nevent: future_event\ndata: ignored\n\ndata: {\ndata: \"type\": \"ping\"\ndata: }\n\n",Provider::Anthropic,&mut |_|panic!("unknown frame emitted text")).unwrap();
    assert_eq!(
        stream_error(
            Provider::OpenAi,
            &json!({"error":{"code":"insufficient_quota"}})
        ),
        SendError::Quota
    );
    assert_eq!(
        stream_error(
            Provider::Anthropic,
            &json!({"error":{"type":"billing_error"}})
        ),
        SendError::Quota
    );
}
