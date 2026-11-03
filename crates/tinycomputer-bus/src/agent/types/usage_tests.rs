//! Serialization and accumulation tests for task model usage.

use super::*;
use serde_json::json;

#[test]
fn keeps_unreported_usage_unknown_and_accumulates_reported_values() {
    let mut usage = ModelUsage::default();
    usage.record(Some(10), Some(4), Some(3), Some(12));
    usage.record(None, None, None, None);
    usage.record(Some(8), Some(2), Some(5), Some(7));

    assert_eq!(usage.calls, 3);
    assert_eq!(usage.input_tokens, Some(18));
    assert_eq!(usage.cached_input_tokens, Some(6));
    assert_eq!(usage.output_tokens, Some(8));
    assert_eq!(usage.cost_usd_nanos, Some(19));
}

#[test]
fn omits_unreported_measurements_from_the_wire() -> Result<(), serde_json::Error> {
    let wire = serde_json::to_value(TaskUsage {
        jev: ModelUsage {
            calls: 1,
            ..ModelUsage::default()
        },
        ..TaskUsage::default()
    })?;
    assert_eq!(wire, json!({"jev": {"calls": 1}}));

    let usage = serde_json::from_value::<ModelUsage>(json!({"calls": 1}))?;
    assert_eq!(
        usage,
        ModelUsage {
            calls: 1,
            ..ModelUsage::default()
        }
    );
    Ok(())
}
