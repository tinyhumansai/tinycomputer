//! Opaque secret facts are typed by the host and redacted from every
//! observation the model and report can see.

use super::*;
use tinycomputer_bus::agent::SecretHandle;

const CARD: &str = "4111 1111 1111 1111";

#[derive(Clone)]
struct TestSecretHost {
    app: App,
    refuse: bool,
    calls: Arc<Mutex<Vec<crate::SecretTypeRequest>>>,
}

impl SecretHost for TestSecretHost {
    fn type_secret(&self, request: &crate::SecretTypeRequest) -> Result<(), String> {
        self.calls.lock().unwrap().push(request.clone());
        if self.refuse {
            return Err("host refused the card field".to_owned());
        }
        let mut sim = self.app.sim();
        sim.fields.insert("To".to_owned(), CARD.to_owned());
        sim.quirks.insert(Quirk::HostSecretEcho);
        Ok(())
    }

    fn redact(&self, _task_id: &str, text: &str) -> String {
        text.replace(CARD, "•••• 1111")
    }
}

fn card_flow() -> Value {
    json!({
        "app": "Mail",
        "steps": [
            {"open": "Mail"},
            "start a new email message",
            {"enter": {"To": "${card number}"}}
        ]
    })
}

fn with_card_handle(request: &mut RunFlowRequest) {
    request.facts.insert("card number".to_owned());
    request.fact_handles.insert(
        "card number".to_owned(),
        SecretHandle {
            handle: "vault-card-1".to_owned(),
            mask: "•••• 1111".to_owned(),
            kind: "payment_card".to_owned(),
        },
    );
}

fn host(
    app: &App,
    refuse: bool,
) -> (
    Arc<TestSecretHost>,
    Arc<Mutex<Vec<crate::SecretTypeRequest>>>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Arc::new(TestSecretHost {
            app: app.clone(),
            refuse,
            calls: calls.clone(),
        }),
        calls,
    )
}

#[tokio::test]
async fn host_types_card_handle_and_echoes_are_redacted_from_model_and_report() {
    let app = App::default();
    let (host, calls) = host(&app, false);
    let run = run_with_secret_host(
        app,
        card_flow(),
        with_card_handle,
        Some(host),
        |id, question, _| (id == "slot_0").then(|| pick(question, "To", 0.99)),
    )
    .await;
    assert_eq!(
        run.result.stop,
        tinycomputer_bus::FlowStopReason::Completed,
        "{}",
        serde_json::to_string(&run.result).unwrap()
    );

    assert_eq!(
        run.app.sim().fields.get("To").map(String::as_str),
        Some(CARD)
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].handle, "vault-card-1");
    assert_eq!(calls[0].field_role, "textfield");
    assert_eq!(calls[0].field_name.as_deref(), Some("To"));
    assert_eq!(calls[0].task_id, "t-secret");
    assert_eq!(calls[0].current_url, "https://shop.test/pay");
    assert!(calls[0].actions_remaining > 0);

    let model = serde_json::to_string(&run.requests).unwrap();
    assert!(!model.contains(CARD));
    assert!(model.contains("•••• 1111"));
    let trace = serde_json::to_string(&run.result.trace).unwrap();
    let learned = serde_json::to_string(&run.result.learned).unwrap();
    let report = serde_json::to_string(&run.result).unwrap();
    assert!(!trace.contains(CARD));
    assert!(!learned.contains(CARD));
    assert!(!report.contains(CARD));
}

#[tokio::test]
async fn host_refusal_preserves_its_reason_and_types_nothing() {
    let app = App::default();
    let (host, calls) = host(&app, true);
    let run = run_with_secret_host(
        app,
        card_flow(),
        with_card_handle,
        Some(host),
        |id, question, _| (id == "slot_0").then(|| pick(question, "To", 0.99)),
    )
    .await;

    assert!(!run.app.sim().fields.contains_key("To"));
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(
        run.result
            .steps
            .iter()
            .any(|step| step.note.contains("host refused the card field"))
    );
}

#[tokio::test]
async fn opaque_handle_without_a_host_never_types_the_handle_or_mask() {
    let run = run_with(
        App::default(),
        card_flow(),
        with_card_handle,
        |id, question, _| (id == "slot_0").then(|| pick(question, "To", 0.99)),
    )
    .await;
    assert!(!run.app.sim().fields.contains_key("To"));
    let report = serde_json::to_string(&run.result).unwrap();
    assert!(report.contains("configured host"));
    assert!(!report.contains("vault-card-1"));
    assert!(!report.contains("•••• 1111"));
}

#[tokio::test]
async fn secret_type_fails_closed_when_the_action_budget_is_spent() {
    let app = App::default();
    let (host, calls) = host(&app, false);
    let run = run_with_secret_host(
        app,
        card_flow(),
        |request| {
            with_card_handle(request);
            request.max_actions = 2;
        },
        Some(host),
        |id, question, _| (id == "slot_0").then(|| pick(question, "To", 0.99)),
    )
    .await;

    assert!(!run.app.sim().fields.contains_key("To"));
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        run.result
            .steps
            .iter()
            .any(|step| step.note.contains("action budget ran out"))
    );
}

#[tokio::test]
async fn secret_type_fails_closed_when_the_live_url_is_unavailable() {
    let app = App::quirky(Quirk::NoURL);
    let (host, calls) = host(&app, false);
    let run = run_with_secret_host(
        app,
        card_flow(),
        with_card_handle,
        Some(host),
        |id, question, _| (id == "slot_0").then(|| pick(question, "To", 0.99)),
    )
    .await;

    assert!(!run.app.sim().fields.contains_key("To"));
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        run.result
            .steps
            .iter()
            .any(|step| step.note.contains("current page URL is unknown"))
    );
}

#[test]
fn structured_outbound_values_are_redacted_recursively() {
    let mut value = json!({
        "page": [CARD, {"field": CARD}],
        "4111111111111111": "number key",
        "4111 1111 1111 1111": "card key",
        "numeric_echo": 4_111_111_111_111_111_u64,
        "count": 1,
    });
    super::super::secret::redact_value(&mut value, &|text| {
        text.replace(CARD, "[masked]")
            .replace("4111111111111111", "[masked]")
    });
    assert_eq!(value["page"][0], "[masked]");
    assert_eq!(value["page"][1]["field"], "[masked]");
    assert_eq!(value["count"], 1);
    assert_eq!(value["numeric_echo"], "[masked]");
    let collisions = ["[masked]", "[masked]#2"].map(|key| value[key].as_str().unwrap());
    assert_eq!(
        std::collections::BTreeSet::from(collisions),
        std::collections::BTreeSet::from(["number key", "card key"])
    );
    assert!(!value.to_string().contains("4111111111111111"));
}
