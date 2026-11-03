//! Tests for queued callback delivery and trace-sink shutdown.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_queued_event_is_rejected_without_a_late_host_write()
-> Result<(), Box<dyn std::error::Error>> {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let host = BlockingRecordingHost {
        events: Arc::default(),
        started: Arc::new(std::sync::Mutex::new(Some(started_tx))),
        release: Arc::new(std::sync::Mutex::new(Some(release_rx))),
    };
    host_connection
        .serve_at(trace_sink::OBJECT_PATH.try_into()?, host.clone())
        .await?;
    host_connection.request_name(trace_sink::INTERFACE).await?;
    let module = Connection::connect(bus.connect().await?).await?;
    let link = TraceLink::new_for_bus_tests(module, tinybus::BusName::new(trace_sink::INTERFACE)?)?;
    let (first_reply, first_result) = mpsc::sync_channel(1);
    send(
        &link.worker,
        Command::Append {
            record: serde_json::json!({"event": "blocker"}),
            deadline: Instant::now() + Duration::from_secs(10),
            byte_budget: 4096,
            reply: first_reply,
        },
    )?;
    tokio::time::timeout(Duration::from_secs(5), started_rx).await??;

    let (queued_reply, queued_result) = mpsc::sync_channel(1);
    let queued_deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .unwrap_or_else(Instant::now);
    send(
        &link.worker,
        Command::Append {
            record: serde_json::json!({"event": "expired"}),
            deadline: queued_deadline,
            byte_budget: 4096,
            reply: queued_reply,
        },
    )?;
    release_tx
        .send(())
        .map_err(|()| TinyBusError::failed("test release receiver was dropped"))?;

    let first_ack =
        tokio::task::spawn_blocking(move || first_result.recv_timeout(Duration::from_secs(5)))
            .await??;
    let queued_ack =
        tokio::task::spawn_blocking(move || queued_result.recv_timeout(Duration::from_secs(5)))
            .await??;

    assert!(first_ack.is_ok());
    assert!(queued_ack.is_err());
    assert_eq!(
        host.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_slice(),
        &[serde_json::json!({"event": "blocker"})],
        "the expired queued event must not be written late"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_joins_an_active_host_callback_and_closes_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let host = BlockingRecordingHost {
        events: Arc::default(),
        started: Arc::new(std::sync::Mutex::new(Some(started_tx))),
        release: Arc::new(std::sync::Mutex::new(Some(release_rx))),
    };
    host_connection
        .serve_at(trace_sink::OBJECT_PATH.try_into()?, host)
        .await?;
    host_connection.request_name(trace_sink::INTERFACE).await?;
    let module = Connection::connect(bus.connect().await?).await?;
    let link = TraceLink::new_for_bus_tests(module, tinybus::BusName::new(trace_sink::INTERFACE)?)?;
    let callback_link = link.clone();
    let callback_deadline = TraceSinkDeadline::after(5_000)?;
    let callback = tokio::task::spawn_blocking(move || {
        callback_link.append(
            &serde_json::json!({"event": "blocker"}),
            callback_deadline,
            4096,
        )
    });
    tokio::time::timeout(Duration::from_secs(5), started_rx).await??;

    let shutdown_link = link.clone();
    let shutdown = tokio::task::spawn_blocking(move || shutdown_link.shutdown());
    while link
        .worker
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some()
    {
        tokio::task::yield_now().await;
    }
    assert!(
        !shutdown.is_finished(),
        "shutdown must wait for the active host callback"
    );
    release_tx
        .send(())
        .map_err(|()| TinyBusError::failed("test release receiver was dropped"))?;

    shutdown.await??;
    assert!(callback.await?.is_ok());
    assert!(
        link.append(
            &serde_json::json!({"event": "after-shutdown"}),
            TraceSinkDeadline::after(5_000)?,
            4096,
        )
        .is_err()
    );
    Ok(())
}
