//! Tests for callback deadlines and expired queue entries.

use super::*;

#[test]
fn an_expired_deadline_is_never_renewed() {
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .unwrap_or_else(Instant::now);

    assert!(remaining_timeout(deadline).is_none());
    assert!(remaining_millis(deadline).is_none());
}

#[test]
fn artifacts_over_the_transport_safe_limit_are_refused_before_queueing() {
    let limit = tinycomputer_bus::trace_sink::MAX_TRACE_ARTIFACT_BYTES;

    assert!(validate_artifact_size(limit).is_ok());
    assert!(validate_artifact_size(limit + 1).is_err());
}

#[test]
fn callback_timeout_is_capped_without_extending_the_operation_deadline() {
    let deadline = Instant::now() + CALL_TIMEOUT * 2;
    assert!(remaining_timeout(deadline).is_some());
    let timeout = remaining_timeout(deadline).unwrap_or_default();

    assert!(timeout <= CALL_TIMEOUT);
    assert!(deadline > Instant::now());
}

#[test]
fn overlapping_callbacks_are_refused_until_the_outer_delivery_finishes() {
    let active = AtomicBool::new(false);

    {
        let outer = CallbackGuard::try_acquire(&active);
        assert!(outer.is_some());
        let Some(_outer) = outer else {
            return;
        };
        assert!(CallbackGuard::try_acquire(&active).is_none());
    }

    assert!(CallbackGuard::try_acquire(&active).is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_already_expired_queued_event_is_rejected_without_a_host_write()
-> Result<(), Box<dyn std::error::Error>> {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    let host = RecordingHost::default();
    host_connection
        .serve_at(trace_sink::OBJECT_PATH.try_into()?, host.clone())
        .await?;
    host_connection.request_name(trace_sink::INTERFACE).await?;
    let module = Connection::connect(bus.connect().await?).await?;
    let link = TraceLink::new_for_bus_tests(module, tinybus::BusName::new(trace_sink::INTERFACE)?)?;
    let (reply, result) = mpsc::sync_channel(1);
    send(
        &link.worker,
        Command::Append {
            record: serde_json::json!({"event": "expired"}),
            deadline: Instant::now()
                .checked_sub(Duration::from_millis(1))
                .unwrap_or_else(Instant::now),
            byte_budget: 4096,
            reply,
        },
    )?;
    let acknowledgment =
        tokio::task::spawn_blocking(move || result.recv_timeout(Duration::from_secs(1))).await??;

    assert!(acknowledgment.is_err());
    assert!(host.0.lock().map_err(|error| error.to_string())?.is_empty());
    Ok(())
}
