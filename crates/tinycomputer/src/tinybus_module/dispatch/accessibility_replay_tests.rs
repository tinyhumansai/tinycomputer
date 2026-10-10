//! Reliable batches, terminal admission fencing and cleanup retry behavior.
#![allow(
    clippy::expect_used,
    reason = "controlled listener fixtures assert their outcomes"
)]
use super::fixture::{Fixture, StartGate};
use super::*;
use std::sync::{Arc, atomic::Ordering};
#[test]
fn discarded_globe_batch_reply_replays_until_acknowledged() {
    use tinycomputer_bus::accessibility::GlobeRead;
    let fixture = Arc::new(Fixture::default());
    let access = Access::new(fixture.clone());
    let started = access.start().expect("start");
    let request = GlobeRead {
        handle: started.handle.clone(),
        acknowledged_batch: None,
    };
    let discarded = access.read(&request).expect("first batch");
    let retried = access.read(&request).expect("replayed batch");
    assert_eq!(discarded.batch, retried.batch);
    assert_eq!(discarded.events, retried.events);
    assert_eq!(
        fixture
            .calls
            .lock()
            .expect("calls")
            .iter()
            .filter(|call| **call == "read")
            .count(),
        1
    );
    let next = access
        .read(&GlobeRead {
            handle: started.handle.clone(),
            acknowledged_batch: Some(discarded.batch),
        })
        .expect("next batch");
    assert_eq!(next.batch, discarded.batch + 1);
    let retry_ack = access
        .read(&GlobeRead {
            handle: started.handle.clone(),
            acknowledged_batch: Some(discarded.batch),
        })
        .expect("repeated acknowledgement");
    assert_eq!(retry_ack.batch, next.batch);
    access.stop(&started.handle).expect("stop");
    assert!(matches!(access.read(&request), Err(Error::UnknownListener)));
}

#[test]
fn reliable_globe_read_reports_overflow_and_legacy_poll_gaps_without_losing_replay() {
    use tinycomputer_bus::accessibility::GlobeRead;
    let fixture = Arc::new(Fixture::default());
    let access = Access::new(fixture.clone());
    let started = access.start().expect("start");
    fixture.overflow.store(true, Ordering::SeqCst);
    let request = GlobeRead {
        handle: started.handle.clone(),
        acknowledged_batch: None,
    };
    let first = access.read(&request).expect("overflow");
    assert!(first.overflow);
    fixture.overflow.store(false, Ordering::SeqCst);
    assert!(access.read(&request).expect("replayed reset").overflow);
    access.poll(&started.handle).expect("legacy drain");
    let next = access
        .read(&GlobeRead {
            acknowledged_batch: Some(first.batch),
            ..request.clone()
        })
        .expect("gap");
    assert!(next.overflow);
    assert!(
        access
            .read(&GlobeRead {
                acknowledged_batch: Some(next.batch + 1),
                ..request.clone()
            })
            .is_err()
    );
    assert_eq!(
        access.read(&request).expect("unchanged snapshot").batch,
        next.batch
    );
    access.shutdown().expect("joined shutdown");
    assert!(access.start().is_err());
}

#[test]
fn failed_native_cleanup_retains_the_lease_and_failed_start_remains_shutdown_reachable() {
    let fixture = Arc::new(Fixture::default());
    let access = Access::new(fixture.clone());
    let started = access.start().expect("start");
    fixture.fail_stop.store(true, Ordering::SeqCst);
    assert!(access.stop(&started.handle).is_err());
    access.poll(&started.handle).expect("lease retained");
    access.stop(&started.handle).expect("retry cleanup");
    assert!(matches!(
        access.poll(&started.handle),
        Err(Error::UnknownListener)
    ));
    fixture.fail_start.store(true, Ordering::SeqCst);
    fixture.fail_stop.store(true, Ordering::SeqCst);
    assert!(access.start().is_err());
    assert!(access.native_owned.load(Ordering::SeqCst));
    access.shutdown().expect("partial startup cleanup");
    assert!(!access.native_owned.load(Ordering::SeqCst));
    assert!(access.start().is_err());
}

#[tokio::test]
async fn canceled_start_waiter_cannot_publish_after_terminal_shutdown() {
    let fixture = Arc::new(Fixture::default());
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *fixture.start_gate.lock().expect("gate") = Some(StartGate {
        entered: entered.clone(),
        release: release.clone(),
    });
    let access = Arc::new(Access::new(fixture.clone()));
    let pending = tokio::spawn(run(access.clone(), "globe-start", Access::start));
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .expect("native start entered");
    pending.abort();
    let _ = pending.await;
    access.close_admission();
    let shutdown = tokio::spawn(run(access.clone(), "globe-shutdown", Access::shutdown));
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .expect("release native start");
    let closed = shutdown.await.expect("cleanup task").expect("reply");
    assert!(closed.ok);
    assert!(access.listener.lock().expect("leases").is_none());
    assert!(!access.native_owned.load(Ordering::SeqCst));
    assert!(access.start().is_err());
    assert_eq!(
        *fixture.calls.lock().expect("calls"),
        ["start", "stop", "stop"]
    );
}

#[test]
fn retired_listener_leases_do_not_exhaust_admission_or_receive_successor_events() {
    use tinycomputer_bus::accessibility::GlobeRead;
    let fixture = Arc::new(Fixture::default());
    let access = Access::new(fixture);
    let first = access.start().expect("first").handle;
    access.stop(&first).expect("stop first");
    for _ in 0..5000 {
        let handle = access.start().expect("new lease").handle;
        assert_ne!(handle, first);
        assert!(matches!(
            access.read(&GlobeRead {
                handle: first.clone(),
                acknowledged_batch: None
            }),
            Err(Error::UnknownListener)
        ));
        access.stop(&handle).expect("stop");
    }
    access.shutdown().expect("terminal cleanup");
}
