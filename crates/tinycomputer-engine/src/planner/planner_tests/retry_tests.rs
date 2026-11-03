//! Tests for trying a hosted model call again: a gateway's passing failure
//! is tried again, a refusal is not, and the tries are bounded.

use std::cell::Cell;

use tinyinference_llm::Error;
use tinyinference_llm::model::ProviderError;

use super::super::ReportedUsage;
use super::super::hosted::{passing, reported_usage, with_retries, with_retries_observed};
use tinyinference_llm::usage::Usage;

fn bad_gateway() -> Error {
    Error::Model("tinyhumans returned HTTP 502: error code: 502".to_owned())
}

#[test]
fn absent_provider_counts_stay_unknown_instead_of_becoming_zero() {
    assert_eq!(reported_usage(None), ReportedUsage::default());
    assert_eq!(
        reported_usage(Some(&Usage {
            input_tokens: 10,
            output_tokens: 2,
            charged_amount_nanos: Some(0),
            ..Usage::default()
        })),
        ReportedUsage {
            input_tokens: Some(10),
            cached_input_tokens: None,
            output_tokens: Some(2),
            cost_usd_nanos: Some(0),
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_model_call_is_tried_again_after_a_gateway_error() {
    // Live, one 502 from Tiny Humans' gateway ended a task at its plan.
    let tries = Cell::new(0);
    let started = tokio::time::Instant::now();
    let answer = with_retries(|| {
        tries.set(tries.get() + 1);
        let failing = tries.get() < 3;
        async move {
            if failing {
                Err(bad_gateway())
            } else {
                Ok("the plan")
            }
        }
    })
    .await;
    assert_eq!(answer.unwrap(), "the plan");
    assert_eq!(tries.get(), 3);
    assert_eq!(
        started.elapsed(),
        std::time::Duration::from_secs(3),
        "1 s, then 2 s apart"
    );
}

#[tokio::test(start_paused = true)]
async fn retry_observer_counts_each_provider_request() {
    let tries = Cell::new(0);
    let recorded = Cell::new(0);
    let (answer, attempts) = with_retries_observed(
        || {
            tries.set(tries.get() + 1);
            let failing = tries.get() < 3;
            async move {
                if failing {
                    Err(bad_gateway())
                } else {
                    Ok("the answer")
                }
            }
        },
        || recorded.set(recorded.get() + 1),
    )
    .await;
    assert_eq!(answer.unwrap(), "the answer");
    assert_eq!(attempts, 3);
    assert_eq!(recorded.get(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_model_call_that_keeps_failing_gives_up_after_its_tries() {
    let tries = Cell::new(0);
    let answer = with_retries(|| {
        tries.set(tries.get() + 1);
        async { Err::<(), _>(bad_gateway()) }
    })
    .await;
    assert!(answer.is_err());
    assert_eq!(tries.get(), 4);
}

#[tokio::test(start_paused = true)]
async fn a_refused_model_call_is_not_tried_again() {
    for refusal in [
        Error::Model("tinyhumans returned HTTP 401: invalid key".to_owned()),
        Error::Validation("network-backed model calls are denied".to_owned()),
    ] {
        let message = refusal.to_string();
        let mut refusal = Some(refusal);
        let tries = Cell::new(0);
        let answer = with_retries(|| {
            tries.set(tries.get() + 1);
            let error = refusal.take().expect("asked once");
            async move { Err::<(), _>(error) }
        })
        .await;
        assert!(answer.is_err());
        assert_eq!(tries.get(), 1, "{message}");
    }
}

#[test]
fn a_failure_passes_by_its_kind_and_what_the_provider_says() {
    assert!(passing(&bad_gateway()));
    assert!(passing(&Error::Model(
        "connection reset by peer".to_owned()
    )));
    let unavailable = ProviderError {
        provider: "openai".to_owned(),
        status: Some(503),
        message: "upstream unavailable".to_owned(),
        retryable: true,
        ..ProviderError::default()
    };
    assert!(passing(&Error::Provider(Box::new(unavailable.clone()))));
    let final_word = ProviderError {
        retryable: false,
        ..unavailable
    };
    assert!(
        !passing(&Error::Provider(Box::new(final_word))),
        "a provider that says it will not pass is believed"
    );
    assert!(!passing(&Error::Unsupported("tools".to_owned())));
}
