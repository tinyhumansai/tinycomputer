//! Linked `TinyBus` fixture tests, serialized around their shared static state.

use super::*;

#[derive(Default)]
struct TestModuleState {
    config: tokio::sync::RwLock<ComputerConfig>,
    applied: StdMutex<Vec<ComputerConfig>>,
    observed: StdMutex<Vec<ObservedCall>>,
    focus_gate: tokio::sync::Mutex<Option<FocusGate>>,
    run_goal_gate: tokio::sync::Mutex<Option<FocusGate>>,
    name_acquired: std::sync::atomic::AtomicBool,
}

struct FocusGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

static LINKED_STATE: OnceLock<StdMutex<Option<Arc<TestModuleState>>>> = OnceLock::new();
static LINKED_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn linked_state_slot() -> &'static StdMutex<Option<Arc<TestModuleState>>> {
    LINKED_STATE.get_or_init(|| StdMutex::new(None))
}

fn linked_setup_factory()
-> impl Fn(Connection, ComputerConfig) -> Pin<Box<dyn Future<Output = tinybus::Result<()>> + Send>>
+ Clone
+ Send
+ Sync
+ 'static {
    let state = Arc::new(TestModuleState::default());
    *linked_state_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&state));
    move |connection, config| {
        let state = Arc::clone(&state);
        Box::pin(async move {
            state
                .applied
                .lock()
                .map_err(|_| TinyBusError::failed("test configuration state poisoned"))?
                .push(config.clone());
            *state.config.write().await = config;
            connection
                .serve_at(
                    ObjectPath::new(names::OBJECT_PATH)?,
                    LinkedService(Arc::clone(&state)),
                )
                .await?;
            if !state
                .name_acquired
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                connection.request_name(names::INTERFACE).await?;
            }
            Ok(())
        })
    }
}

mod linked_service {
    fn setup_factory() -> impl Fn(
        tinybus::Connection,
        tinycomputer_bus::config::ComputerConfig,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = tinybus::Result<()>> + Send>,
    > + Clone
    + Send
    + Sync {
        super::linked_setup_factory()
    }

    tinybus_module::module_export_static! {
        setup_factory = setup_factory,
        config = tinycomputer_bus::config::ComputerConfig,
        worker_threads = 1,
        provides = ["ai.tinyhumans.tinycomputer.Desktop"],
        methods = ["Describe", "AccessibilityFocus", "RunGoal"],
        signals = [],
        requires = [],
        optional = [],
        lazy = false,
    }
}

struct LinkedService(Arc<TestModuleState>);

#[async_trait]
impl Interface for LinkedService {
    fn name(&self) -> InterfaceName {
        static_contract_value(InterfaceName::new(names::INTERFACE))
    }

    fn members(&self) -> Vec<MemberName> {
        [
            task_names::methods::DESCRIBE,
            names::accessibility::ACCESSIBILITY_FOCUS,
            names::methods::RUN_GOAL,
        ]
        .iter()
        .map(|member| static_contract_value(MemberName::new(*member)))
        .collect()
    }

    fn requires_confidential(&self, member: &MemberName) -> bool {
        handler_requires_confidential(member.as_str())
    }

    async fn call(&self, member: &MemberName, args: Value) -> tinybus::Result<Value> {
        self.reply(member, &args).await
    }

    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        context: &tinybus::service::CallContext,
    ) -> tinybus::Result<Value> {
        self.0
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ObservedCall {
                member: member.as_str().to_owned(),
                confidential: context.is_confidential(),
                args: args.clone(),
            });
        self.reply(member, &args).await
    }
}

impl LinkedService {
    async fn reply(&self, member: &MemberName, args: &Value) -> tinybus::Result<Value> {
        if member.as_str() == task_names::methods::DESCRIBE
            && !args.as_array().is_some_and(Vec::is_empty)
        {
            return Err(TinyBusError::failed("Describe takes no arguments"));
        }

        if member.as_str() == names::accessibility::ACCESSIBILITY_FOCUS {
            if let Some(gate) = self.0.focus_gate.lock().await.take() {
                let _ = gate.entered.send(());
                let _ = gate.release.await;
            }
            return serde_json::to_value(DesktopResponse::ok(
                member.as_str(),
                serde_json::to_value(FocusedTextContext {
                    app_name: Some("linked fixture".to_owned()),
                    role: Some("text".to_owned()),
                    text: "attested result".to_owned(),
                    selected_text: None,
                    raw_error: None,
                    bounds: None,
                })
                .map_err(|error| TinyBusError::failed(error.to_string()))?,
            ))
            .map_err(|error| TinyBusError::failed(error.to_string()));
        }

        let config = self.0.config.read().await.clone();
        if member.as_str() == names::methods::RUN_GOAL {
            if let Some(gate) = self.0.run_goal_gate.lock().await.take() {
                let _ = gate.entered.send(());
                let _ = gate.release.await;
            }
            let request = args
                .as_array()
                .and_then(|items| items.first())
                .cloned()
                .unwrap_or(Value::Null);
            return response(member.as_str(), &request, &config);
        }

        serde_json::to_value(test_capabilities(&config))
            .map_err(|error| TinyBusError::failed(error.to_string()))
    }
}

struct LinkedFixture {
    client: ComputerClient,
    state: Arc<TestModuleState>,
    _host: Arc<ModuleHost>,
    _broker_task: tokio::task::JoinHandle<tinybus::Result<()>>,
}

fn linked_fixture(initial: &ComputerConfig) -> Result<LinkedFixture, Box<dyn std::error::Error>> {
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = Arc::new(ModuleHost::new(broker));
    let initial_value = serde_json::to_value(initial)?;
    host.attach_linked_with_config(linked_service::linked_module()?, initial_value)?;
    let state_guard = linked_state_slot()
        .lock()
        .map_err(|_| std::io::Error::other("linked test state poisoned"))?;
    let state = Arc::clone(
        state_guard
            .as_ref()
            .ok_or_else(|| std::io::Error::other("linked setup did not publish its test state"))?,
    );
    let client = client_for(&bus, Some(Arc::clone(&host)));
    client.record_loaded_with(initial)?;
    Ok(LinkedFixture {
        client,
        state,
        _host: host,
        _broker_task: broker_task,
    })
}

#[tokio::test(flavor = "current_thread")]
async fn describe_uses_the_zero_argument_wire_shape() -> Result<(), Box<dyn std::error::Error>> {
    let _serial = LINKED_TEST_LOCK.lock().await;
    let config = ComputerConfig::default();
    let fixture = linked_fixture(&config)?;

    let described = fixture.client.describe(&config).await?;

    assert_eq!(
        described.contract_version,
        tinycomputer_bus::CONTRACT_VERSION
    );
    let describe_call = fixture
        .state
        .observed
        .lock()
        .map_err(|_| std::io::Error::other("observed calls poisoned"))?
        .iter()
        .find(|call| call.member == task_names::methods::DESCRIBE)
        .cloned()
        .ok_or_else(|| std::io::Error::other("Describe call was not observed"))?;
    assert_eq!(describe_call.args, Value::Array(Vec::new()));
    Ok(())
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn run_goal_remains_pending_past_the_default_bus_timeout()
-> Result<(), Box<dyn std::error::Error>> {
    let _serial = LINKED_TEST_LOCK.lock().await;
    let config = ComputerConfig::default();
    let fixture = linked_fixture(&config)?;
    let (entered_tx, mut entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    *fixture.state.run_goal_gate.lock().await = Some(FocusGate {
        entered: entered_tx,
        release: release_rx,
    });
    let keep_clock_scheduled = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });

    let client = fixture.client.clone();
    let call_config = config.clone();
    let mut call = tokio::spawn(async move {
        client
            .call_desktop::<_, String>(
                &call_config,
                names::methods::RUN_GOAL,
                json!({"goal": "wait for the fixture"}),
            )
            .await
    });
    tokio::select! {
        biased;
        result = &mut call => {
            keep_clock_scheduled.abort();
            return Err(std::io::Error::other(format!(
                "RunGoal call finished before reaching its service gate: {result:?}"
            )).into());
        }
        result = &mut entered_rx => {
            result.map_err(|_| std::io::Error::other("RunGoal service exited before the gate"))?;
        }
    }

    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    assert!(
        !call.is_finished(),
        "RunGoal must use its extended deadline instead of TinyBus' default"
    );

    release_tx
        .send(())
        .map_err(|()| std::io::Error::other("RunGoal gate closed before release"))?;
    let result = call.await?;
    keep_clock_scheduled.abort();
    let _ = keep_clock_scheduled.await;
    assert_eq!(result?, names::methods::RUN_GOAL);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn attested_client_keeps_configuration_exclusive_through_member_completion()
-> Result<(), Box<dyn std::error::Error>> {
    let _serial = LINKED_TEST_LOCK.lock().await;
    let first_config = ComputerConfig {
        jev: Some(JevConfig::new("fixture-secret")),
        ..ComputerConfig::default()
    };
    let second_config = ComputerConfig::default();
    let fixture = linked_fixture(&first_config)?;
    assert!(format!("{:?}", fixture.client).contains("ComputerClient"));

    let described = fixture.client.describe(&first_config).await?;
    assert!(described.jev_configured);
    assert_eq!(
        fixture
            .state
            .applied
            .lock()
            .map_err(|_| std::io::Error::other("applied config log poisoned"))?
            .len(),
        1
    );

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    *fixture.state.focus_gate.lock().await = Some(FocusGate {
        entered: entered_tx,
        release: release_rx,
    });
    let first_client = fixture.client.clone();
    let call_config = first_config.clone();
    let focus = tokio::spawn(async move {
        first_client
            .call_desktop::<_, FocusedTextContext>(
                &call_config,
                names::accessibility::ACCESSIBILITY_FOCUS,
                FocusQuery::default(),
            )
            .await
    });
    entered_rx.await?;

    // A call with the same configuration runs alongside the one in flight.
    let alongside = fixture.client.describe(&first_config).await?;
    assert!(alongside.jev_configured);

    let second_client = fixture.client.clone();
    let next_config = second_config.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let next = tokio::spawn(async move {
        let _ = started_tx.send(());
        second_client.describe(&next_config).await
    });
    started_rx.await?;
    tokio::task::yield_now().await;
    assert!(
        !next.is_finished(),
        "second call waits for the active guard"
    );
    assert_eq!(
        fixture
            .state
            .applied
            .lock()
            .map_err(|_| std::io::Error::other("applied config log poisoned"))?
            .len(),
        1
    );

    release_tx
        .send(())
        .map_err(|()| std::io::Error::other("focus gate closed before release"))?;
    let focused = focus.await??;
    assert_eq!(focused.text, "attested result");
    let focus_call = fixture
        .state
        .observed
        .lock()
        .map_err(|_| std::io::Error::other("observed calls poisoned"))?
        .iter()
        .find(|call| call.member == names::accessibility::ACCESSIBILITY_FOCUS)
        .cloned()
        .ok_or_else(|| std::io::Error::other("linked focus call was not observed"))?;
    assert!(focus_call.confidential);
    assert_eq!(focus_call.args[0]["verbose"], false);
    let reconfigured = next.await??;
    assert!(!reconfigured.jev_configured);
    assert_eq!(
        fixture
            .state
            .applied
            .lock()
            .map_err(|_| std::io::Error::other("applied config log poisoned"))?
            .len(),
        2
    );
    Ok(())
}
