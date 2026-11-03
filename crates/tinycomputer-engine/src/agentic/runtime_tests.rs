//! Host endpoint and Jev usage integration tests.

use std::sync::Arc;

#[cfg(feature = "planner")]
use std::io::{Read, Write};
#[cfg(feature = "planner")]
use std::net::{TcpListener, TcpStream};

use serde_json::json;
use tinycomputer_bus::agentic::JevConfig;
#[cfg(feature = "planner")]
use tinycomputer_bus::agentic::JevProvider;
use tinyinference_decisions::{EvaluationRequest, Noul, Question};

use super::JevRuntime;
use crate::UsageRecorder;

#[cfg(feature = "planner")]
type StubServer = (
    String,
    Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<std::io::Result<Vec<String>>>,
);

#[cfg(feature = "planner")]
#[tokio::test]
async fn host_planner_rescue_output_and_jev_calls_stay_on_the_supplied_stub()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::{Briefing, Harvest, HostModels, openai_compatible_models};
    use tinycomputer_bus::agent::SurfaceKind;

    const PLAN: &str = r#"{"app":"browser","steps":[{"browse":"https://example.com"}]}"#;
    const RESCUE: &str = r#"{"action":"give_up","reason":"no repair"}"#;
    const OUTPUT: &str = r#"{"answer":"done"}"#;

    let _network_test = crate::MODEL_NETWORK_TEST_MUTEX.lock().await;
    let (base, requests, server) = stub_server([PLAN, RESCUE, OUTPUT])?;
    let models = HostModels {
        planner: host_model(&base, "planner-secret", "planner-local"),
        rescue: Some(host_model(&base, "rescue-secret", "rescue-local")),
        output: Some(host_model(&base, "output-secret", "output-local")),
    };
    let debug = format!("{models:?}");
    for secret in ["planner-secret", "rescue-secret", "output-secret"] {
        assert!(!debug.contains(secret));
    }

    let usage = Arc::new(UsageRecorder::default());
    let (planner, rescuer, shaper) =
        openai_compatible_models(&models).map_err(std::io::Error::other)?;
    let planner = planner.with_usage_recorder(Arc::clone(&usage));
    let rescuer = rescuer.with_usage_recorder(Arc::clone(&usage));
    let shaper = shaper.with_usage_recorder(Arc::clone(&usage));

    let (plan, _) = planner
        .plan_measured("open the example page", &[], &[], &[SurfaceKind::Browser])
        .await;
    assert!(plan.is_ok(), "{plan:?}");
    assert!(matches!(
        rescuer
            .guide(&Briefing::default())
            .await
            .map_err(std::io::Error::other)?,
        crate::Guidance::GiveUp { .. }
    ));
    let output = shaper
        .shape(&Harvest::default())
        .await
        .map_err(std::io::Error::other)?;
    assert_eq!(output["answer"], "done");

    let mut jev = JevConfig::new("jev-secret");
    jev.provider = JevProvider::OpenRouter;
    jev.model = Some("jev-latest".to_owned());
    jev.endpoint_url = Some(base.trim_end_matches("/v1").to_owned() + "/jev");
    let runtime = JevRuntime::configure_for_host(&jev)
        .map_err(|error| std::io::Error::other(error.message.clone()))?
        .with_usage_recorder(Arc::clone(&usage));
    runtime.warm(1).await;
    runtime.evaluate(None, &jev_request()).await?;

    let seen = server
        .join()
        .map_err(|_| std::io::Error::other("stub server thread panicked"))??;
    assert_stub_routes(&seen);
    assert_eq!(requests.load(std::sync::atomic::Ordering::Relaxed), 7);
    assert_usage(&usage);
    Ok(())
}

#[tokio::test]
async fn failed_jev_attempt_keeps_its_started_call_in_the_tally()
-> Result<(), Box<dyn std::error::Error>> {
    let usage = Arc::new(UsageRecorder::default());
    let mut config = JevConfig::new("k");
    config.endpoint_url = Some("http://127.0.0.1:1/jev".to_owned());
    config.max_retries = Some(0);
    let runtime = JevRuntime::configure_for_host(&config)
        .map_err(|error| std::io::Error::other(error.message.clone()))?
        .with_usage_recorder(Arc::clone(&usage));
    let _ = runtime.evaluate(None, &jev_request()).await;
    assert_eq!(usage.snapshot().jev.calls, 1);
    Ok(())
}

#[cfg(feature = "planner")]
fn host_model(base_url: &str, api_key: &str, model: &str) -> crate::HostModelConfig {
    crate::HostModelConfig {
        api_key: api_key.to_owned(),
        base_url: base_url.to_owned(),
        model: model.to_owned(),
    }
}

fn jev_request() -> EvaluationRequest {
    EvaluationRequest::new(
        "jev-latest",
        json!("The stub integration test."),
        [(
            "ready".to_owned(),
            Question::Noul(Noul {
                instructions: json!("Is the test request ready?"),
                criteria: None,
            }),
        )]
        .into(),
    )
}

#[cfg(feature = "planner")]
fn assert_stub_routes(requests: &[String]) {
    assert_eq!(requests.len(), 7, "all model calls must hit this stub");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("POST /v1/chat/completions "))
            .count(),
        4
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("POST /jev "))
            .count(),
        3
    );
    for model in ["planner-local", "rescue-local", "output-local"] {
        assert!(
            requests.iter().any(|request| request.contains(model)),
            "no local request used {model}"
        );
    }
}

#[cfg(feature = "planner")]
fn assert_usage(recorder: &UsageRecorder) {
    let tally = recorder.snapshot();
    for (role, calls) in [(tally.planner, 2), (tally.rescue, 1), (tally.output, 1)] {
        assert_eq!(role.calls, calls);
        assert_eq!(role.input_tokens, Some(10));
        assert_eq!(role.cached_input_tokens, Some(2));
        assert_eq!(role.output_tokens, Some(3));
        assert_eq!(role.cost_usd_nanos, Some(400));
    }
    assert_eq!(tally.jev.calls, 3);
    assert_eq!(tally.jev.input_tokens, Some(60));
    assert_eq!(tally.jev.cached_input_tokens, Some(21));
    assert_eq!(tally.jev.output_tokens, Some(15));
    assert_eq!(tally.jev.cost_usd_nanos, Some(1_200));
}

#[cfg(feature = "planner")]
fn stub_server(replies: [&'static str; 3]) -> std::io::Result<StubServer> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let base = format!("http://{address}/v1");
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        let mut planner_attempts = 0;
        let mut chat_replies = 0;
        for _ in 0..7 {
            let (mut stream, _) = listener.accept()?;
            let raw = read_request(&mut stream)?;
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = raw.lines().next().unwrap_or_default();
            if path.starts_with("POST /jev ") {
                let body = json!({
                    "model": "typesafe/jev-v1",
                    "answers": {"ready": {"type": "noul", "noul": 0.99}},
                    "usage": {
                        "input_tokens": 20,
                        "cached_input_tokens": 7,
                        "output_tokens": 5,
                        "cost": 0.000_000_4
                    }
                })
                .to_string();
                write_response(&mut stream, "application/json", &body)?;
            } else {
                if raw.contains("planner-local") && planner_attempts == 0 {
                    planner_attempts += 1;
                    write_response_with_status(
                        &mut stream,
                        503,
                        "application/json",
                        r#"{"error":"temporary outage"}"#,
                    )?;
                    requests.push(raw);
                    continue;
                }
                if raw.contains("planner-local") {
                    planner_attempts += 1;
                }
                let call = replies[chat_replies];
                chat_replies += 1;
                let body = format!(
                    "data: {{\"id\":\"stub\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":{}}},\"finish_reason\":null}}]}}\n\n\
                     data: {{\"id\":\"stub\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":3,\"total_tokens\":13,\"prompt_tokens_details\":{{\"cached_tokens\":2}},\"cost\":0.0000004}}}}\n\n\
                     data: [DONE]\n\n",
                    serde_json::to_string(call).map_err(std::io::Error::other)?
                );
                write_response(&mut stream, "text/event-stream", &body)?;
            }
            requests.push(raw);
        }
        Ok(requests)
    });
    Ok((base, calls, server))
}

#[cfg(feature = "planner")]
fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    let headers = String::from_utf8_lossy(&bytes);
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or_default();
    let offset = bytes.len();
    bytes.resize(offset + length, 0);
    stream.read_exact(&mut bytes[offset..])?;
    String::from_utf8(bytes).map_err(std::io::Error::other)
}

#[cfg(feature = "planner")]
fn write_response(stream: &mut TcpStream, content_type: &str, body: &str) -> std::io::Result<()> {
    write_response_with_status(stream, 200, content_type, body)
}

#[cfg(feature = "planner")]
fn write_response_with_status(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} Stub\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}
