//! In-memory services and common contract assertions.

use super::*;

pub(crate) const TEST_MODULE_ID: &str = env!("CARGO_PKG_NAME");

pub(crate) fn static_contract_value<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            eprintln!("client test failed: {error:?}");
            std::process::exit(1);
        }
    }
}

pub(crate) static TEST_RECORD: ReleaseRecord = ReleaseRecord {
    id: TEST_MODULE_ID,
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.16.0",
    release_url: "https://example.invalid/test",
    assets: &[],
};

#[derive(Default)]
pub(crate) struct SilentSink;

impl FailureSink for SilentSink {
    fn capture(&self, _report: &tinybus::module::failure::FailureReport<'_>) {}
}

pub(crate) struct TestAccess {
    pub(crate) bus: MemoryBus,
    host: Option<Arc<ModuleHost>>,
    reporter: FailureReporter,
}

impl TestAccess {
    fn new(bus: MemoryBus, host: Option<Arc<ModuleHost>>) -> Self {
        Self {
            bus,
            host,
            reporter: FailureReporter::new(Arc::new(SilentSink)),
        }
    }
}

impl ModuleAccess for TestAccess {
    fn find(&self, id: &str) -> Option<&'static ReleaseRecord> {
        (id == TEST_MODULE_ID).then_some(&TEST_RECORD)
    }

    fn enabled(&self) -> bool {
        true
    }

    fn reporter(&self) -> &FailureReporter {
        &self.reporter
    }

    fn ensure_loaded<'a>(&'a self, _id: &'a str) -> AccessFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(host) = &self.host {
                host.ensure_ready(names::INTERFACE)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        })
    }

    fn proxy<'a>(
        &'a self,
        bus_name: &'a str,
        object_path: &'a str,
    ) -> AccessFuture<'a, Result<Proxy, ProxyFailure>> {
        let bus = self.bus.clone();
        let bus_name = bus_name.to_owned();
        let object_path = object_path.to_owned();
        Box::pin(async move {
            let transport = bus.connect().await.map_err(|_| ProxyFailure::Transport)?;
            let connection = Connection::connect(transport)
                .await
                .map_err(|_| ProxyFailure::Transport)?;
            connection
                .proxy(&bus_name, &object_path, &bus_name)
                .map_err(|_| ProxyFailure::Contract)
        })
    }

    fn reinitialize<'a>(
        &'a self,
        id: &'a str,
        config: Value,
    ) -> AccessFuture<'a, Result<(), ReconfigureFailure>> {
        Box::pin(async move {
            let Some(host) = &self.host else {
                return Ok(());
            };
            host.reinitialize(id, config)
                .await
                .map(|_| ())
                .map_err(|_| ReconfigureFailure::Failed)
        })
    }
}

/// A client for a module id the host's release table does not hold.
pub(crate) fn client_for_unknown_module(bus: &MemoryBus) -> ComputerClient {
    ComputerClient::new(ReconfigurableModule::new(
        TestAccess::new(bus.clone(), None),
        "tinycomputer-unknown",
    ))
}

pub(crate) fn client_for(bus: &MemoryBus, host: Option<Arc<ModuleHost>>) -> ComputerClient {
    ComputerClient::new(ReconfigurableModule::new(
        TestAccess::new(bus.clone(), host),
        TEST_MODULE_ID,
    ))
}

pub(crate) fn test_capabilities(config: &ComputerConfig) -> Capabilities {
    Capabilities {
        contract_version: tinycomputer_bus::CONTRACT_VERSION,
        surfaces: Vec::new(),
        jev_configured: config.jev.is_some(),
        planner_configured: config.planner.is_some(),
        rescue_configured: false,
        output_configured: false,
        decision_model: None,
        planner_model: None,
        rescue_model: None,
        output_model: None,
        step_kinds: Vec::new(),
        guide: String::new(),
        members: Vec::new(),
        examples: Vec::new(),
        catalogue: catalogue::summaries(),
    }
}

pub(crate) const FLOW_MEMBERS: &[&str] = &[
    names::methods::RESOLVE_INTENT,
    names::methods::RUN_GOAL,
    names::methods::RUN_FLOW,
    names::methods::VALIDATE_FLOW,
    names::methods::FLOW_GUIDE,
];

pub(crate) fn expected_family(member: &str) -> Family {
    if task_names::METHODS.contains(&member) {
        Family::Task
    } else if tinycomputer_bus::browser::names::METHODS.contains(&member) {
        Family::Browser
    } else if FLOW_MEMBERS.contains(&member) {
        Family::Flow
    } else {
        Family::Desktop
    }
}

pub(crate) fn catalogue_request(member: &str) -> Result<Value, serde_json::Error> {
    Ok(match member {
        tinycomputer_bus::browser::names::methods::NAVIGATE => {
            serde_json::to_value(SessionRequest::new(
                SessionId::new("catalogue-session"),
                NavigateRequest::new("https://example.com"),
            ))?
        }
        tinycomputer_bus::browser::names::methods::CLAIM_PROFILE => {
            serde_json::to_value(ClaimProfileRequest::new("/profile/catalogue", 0))?
        }
        tinycomputer_bus::browser::names::methods::READ_OUTPUT => {
            serde_json::to_value(ReadOutputRequest::new(OutputId::new("catalogue-output")))?
        }
        _ => json!({"member": member}),
    })
}

pub(crate) fn assert_catalogue_reply(member: &str, value: &Value) {
    match member {
        tinycomputer_bus::browser::names::methods::NAVIGATE => {
            assert_eq!(value["url"], "https://example.com");
        }
        tinycomputer_bus::browser::names::methods::CLAIM_PROFILE => {
            assert_eq!(value["outcome"], "claimed");
            assert_eq!(value["path"], "/profile/fixture");
        }
        _ => assert_eq!(*value, json!(member), "member {member} reply"),
    }
}

pub(crate) fn response(
    member: &str,
    request: &Value,
    config: &ComputerConfig,
) -> tinybus::Result<Value> {
    let malformed_version = request
        .get("malformed_version")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let wrong_data = request
        .get("wrong_data")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if member == task_names::methods::DESCRIBE {
        return serde_json::to_value(test_capabilities(config))
            .map_err(|error| TinyBusError::failed(error.to_string()));
    }

    let family = expected_family(member);
    if family == Family::Task {
        if malformed_version {
            return Ok(json!({
                "version": "unsupported",
                "ok": true,
                "data": "unexpected",
                "error": null
            }));
        }
        let data = if wrong_data { json!(17) } else { json!(member) };
        return serde_json::to_value(AgentResponse::ok(data))
            .map_err(|error| TinyBusError::failed(error.to_string()));
    }

    if malformed_version {
        return Ok(json!({
            "version": "unsupported",
            "ok": true,
            "command": member,
            "data": "unexpected",
            "error": null
        }));
    }

    let data = if wrong_data {
        json!(17)
    } else {
        match member {
            "AccessibilityFocus" => serde_json::to_value(FocusedTextContext {
                app_name: Some("fixture".to_owned()),
                role: Some("text".to_owned()),
                text: "typed response".to_owned(),
                selected_text: None,
                raw_error: None,
                bounds: None,
            })
            .map_err(|error| TinyBusError::failed(error.to_string()))?,
            "BrowserClaimProfile" => serde_json::to_value(ClaimProfileResponse {
                outcome: ProfileClaim::Claimed,
                path: Some("/profile/fixture".to_owned()),
            })
            .map_err(|error| TinyBusError::failed(error.to_string()))?,
            "BrowserNavigate" => serde_json::to_value(PageState::new("https://example.com"))
                .map_err(|error| TinyBusError::failed(error.to_string()))?,
            "BrowserReadOutput" => {
                let read: ReadOutputRequest = serde_json::from_value(request.clone())
                    .map_err(|error| TinyBusError::failed(error.to_string()))?;
                serde_json::to_value(OutputChunk {
                    id: read.output,
                    offset: read.offset,
                    data: "aGVsbG8=".to_owned(),
                    eof: true,
                })
                .map_err(|error| TinyBusError::failed(error.to_string()))?
            }
            _ => json!(member),
        }
    };
    serde_json::to_value(DesktopResponse::ok(member, data))
        .map_err(|error| TinyBusError::failed(error.to_string()))
}

pub(crate) struct EchoService {
    config: Arc<tokio::sync::RwLock<ComputerConfig>>,
    pub(crate) calls: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) seen: Arc<StdMutex<Vec<ObservedCall>>>,
}

#[async_trait]
impl Interface for EchoService {
    fn name(&self) -> InterfaceName {
        static_contract_value(InterfaceName::new(names::INTERFACE))
    }

    fn members(&self) -> Vec<MemberName> {
        names::METHODS
            .iter()
            .map(|member| static_contract_value(MemberName::new(*member)))
            .collect()
    }

    fn requires_confidential(&self, member: &MemberName) -> bool {
        handler_requires_confidential(member.as_str())
    }

    async fn call(&self, member: &MemberName, args: Value) -> tinybus::Result<Value> {
        self.invoke(member, args).await
    }

    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        context: &tinybus::service::CallContext,
    ) -> tinybus::Result<Value> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ObservedCall {
                member: member.as_str().to_owned(),
                confidential: context.is_confidential(),
                args: args.clone(),
            });
        self.invoke(member, args).await
    }
}

impl EchoService {
    async fn invoke(&self, member: &MemberName, args: Value) -> tinybus::Result<Value> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let request = args
            .as_array()
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or(Value::Null);
        let config = self.config.read().await.clone();
        response(member.as_str(), &request, &config)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ObservedCall {
    pub(crate) member: String,
    pub(crate) confidential: bool,
    pub(crate) args: Value,
}

pub(crate) fn handler_requires_confidential(member: &str) -> bool {
    task_names::CONFIDENTIAL.contains(&member)
        || [
            names::methods::RESOLVE_INTENT,
            names::methods::RUN_GOAL,
            names::methods::RUN_FLOW,
            names::accessibility::ACCESSIBILITY_FOCUS,
            names::accessibility::ACCESSIBILITY_VALIDATE_TARGET,
            names::accessibility::ACCESSIBILITY_PASTE,
            tinycomputer_bus::browser::names::methods::READ_OUTPUT,
        ]
        .contains(&member)
}

pub(crate) struct UntrustedFixture {
    pub(crate) bus: MemoryBus,
    pub(crate) _broker_task: tokio::task::JoinHandle<tinybus::Result<()>>,
    pub(crate) _service_connection: Connection,
    pub(crate) calls: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) seen: Arc<StdMutex<Vec<ObservedCall>>>,
}

pub(crate) async fn untrusted_fixture() -> Result<UntrustedFixture, Box<dyn std::error::Error>> {
    let bus = MemoryBus::new();
    let broker_task = Broker::new().spawn(bus.clone());
    let transport = bus.connect().await?;
    let service_connection = Connection::connect(transport).await?;
    service_connection.request_name(names::INTERFACE).await?;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = Arc::new(StdMutex::new(Vec::new()));
    service_connection
        .serve_at(
            ObjectPath::new(names::OBJECT_PATH)?,
            EchoService {
                config: Arc::new(tokio::sync::RwLock::new(ComputerConfig::default())),
                calls: Arc::clone(&calls),
                seen: Arc::clone(&seen),
            },
        )
        .await?;
    Ok(UntrustedFixture {
        bus,
        _broker_task: broker_task,
        _service_connection: service_connection,
        calls,
        seen,
    })
}
