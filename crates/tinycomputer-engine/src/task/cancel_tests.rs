//! Tests for tests behavior.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use tinycomputer_bus::agent::{StartTaskRequest, SurfaceKind, TaskConstraints};

use crate::task::{FlowFuture, FlowRunner, Tasks};

struct DropSignal {
    notify: Arc<tokio::sync::Notify>,
    dropped: Arc<AtomicBool>,
}

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
}

struct PendingRunner {
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<tokio::sync::Notify>,
    dropped_flag: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
}

impl FlowRunner for PendingRunner {
    fn run(
        &self,
        _task: &tinycomputer_bus::agent::TaskId,
        _constraints: &TaskConstraints,
        _request: tinycomputer_bus::RunFlowRequest,
    ) -> FlowFuture {
        let started = self.started.clone();
        let dropped = self.dropped.clone();
        let dropped_flag = self.dropped_flag.clone();
        Box::pin(async move {
            started.notify_one();
            let _signal = DropSignal {
                notify: dropped,
                dropped: dropped_flag,
            };
            std::future::pending().await
        })
    }

    fn release(&self, _task: &tinycomputer_bus::agent::TaskId) {
        self.released
            .store(self.dropped_flag.load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

#[tokio::test]
async fn cancel_and_wait_joins_worker_before_releasing_runner() {
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(tokio::sync::Notify::new());
    let dropped_flag = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let tasks = Arc::new(Tasks::new(Arc::new(PendingRunner {
        started: started.clone(),
        dropped: dropped.clone(),
        dropped_flag,
        released: released.clone(),
    })));
    let flow = serde_json::from_value(serde_json::json!({
        "app": "browser",
        "steps": [{"browse": "https://shop.example.com"}]
    }))
    .unwrap();
    let task = tasks
        .start(&StartTaskRequest {
            flow: Some(flow),
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..Default::default()
            },
            ..Default::default()
        })
        .data
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();

    let first = {
        let tasks = tasks.clone();
        let task = task.id.clone();
        tokio::spawn(async move { tasks.cancel_and_wait(&task).await })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    first.abort();
    let _ = first.await;
    let cancelled = tokio::time::timeout(Duration::from_secs(2), tasks.cancel_and_wait(&task.id))
        .await
        .unwrap();

    assert!(cancelled.data.is_some());
    assert!(released.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_previous_generation_cannot_complete_the_current_worker_wait() {
    let (old_tx, old_rx) = tokio::sync::watch::channel(false);
    let (current_tx, mut current_rx) = tokio::sync::watch::channel(false);
    let (finished, mut finished_rx) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn(async move {
        super::wait_for_worker(&mut current_rx).await;
        let _ = finished.send(());
    });

    old_tx.send_replace(true);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut finished_rx)
            .await
            .is_err(),
        "the previous generation must not wake the current worker waiter"
    );
    current_tx.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), finished_rx)
        .await
        .unwrap()
        .unwrap();
    waiter.await.unwrap();
    drop(old_rx);
}
