//! Typed member request and response behavior.

use super::*;

#[tokio::test]
async fn typed_accessibility_browser_profile_task_and_held_output_calls_round_trip()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = untrusted_fixture().await?;
    let client = client_for(&fixture.bus, None);
    let config = ComputerConfig::default();
    client.record_loaded_with(&config)?;

    let focus = client
        .call_desktop::<_, FocusedTextContext>(
            &config,
            names::accessibility::ACCESSIBILITY_FOCUS,
            FocusQuery::default(),
        )
        .await;
    assert!(matches!(focus, Err(ClientError::Bus(_))));

    let claimed = client
        .call_desktop::<_, ClaimProfileResponse>(
            &config,
            tinycomputer_bus::browser::names::methods::CLAIM_PROFILE,
            ClaimProfileRequest::new("/profile/fixture", 0),
        )
        .await?;
    assert_eq!(claimed.outcome, ProfileClaim::Claimed);
    assert_eq!(claimed.path.as_deref(), Some("/profile/fixture"));

    let navigated = client
        .call_desktop::<_, PageState>(
            &config,
            tinycomputer_bus::browser::names::methods::NAVIGATE,
            SessionRequest::new(
                SessionId::new("session-1"),
                NavigateRequest::new("https://example.com"),
            ),
        )
        .await?;
    assert_eq!(navigated.url, "https://example.com");

    let chunk = client
        .call_desktop::<_, OutputChunk>(
            &config,
            tinycomputer_bus::browser::names::methods::READ_OUTPUT,
            ReadOutputRequest::new(OutputId::new("held-image-1")),
        )
        .await?;
    assert_eq!(chunk.id.as_str(), "held-image-1");
    assert_eq!(chunk.offset, 0);
    assert_eq!(chunk.data, "aGVsbG8=");
    assert!(chunk.eof);

    let planned = client
        .call_task::<_, Value>(
            &config,
            task_names::methods::PLAN_TASK,
            tinycomputer_bus::agent::PlanTaskRequest::default(),
        )
        .await?;
    assert_eq!(planned, json!(task_names::methods::PLAN_TASK));

    let seen = fixture
        .seen
        .lock()
        .map_err(|_| std::io::Error::other("untrusted call log poisoned"))?
        .clone();
    for member in [
        tinycomputer_bus::browser::names::methods::CLAIM_PROFILE,
        tinycomputer_bus::browser::names::methods::NAVIGATE,
        tinycomputer_bus::browser::names::methods::READ_OUTPUT,
        task_names::methods::PLAN_TASK,
    ] {
        let observed = seen
            .iter()
            .find(|call| call.member == member)
            .ok_or_else(|| std::io::Error::other("typed member was not observed"))?;
        assert!(!observed.confidential, "{member} is public");
        assert_eq!(observed.args.as_array().map(Vec::len), Some(1));
    }
    let claim = seen
        .iter()
        .find(|call| call.member == tinycomputer_bus::browser::names::methods::CLAIM_PROFILE)
        .ok_or_else(|| std::io::Error::other("claim request was not observed"))?;
    assert_eq!(
        claim.args[0],
        json!({
            "path": "/profile/fixture",
            "wait_ms": 0,
            "fall_back_to_fresh": true
        })
    );
    let navigation = seen
        .iter()
        .find(|call| call.member == tinycomputer_bus::browser::names::methods::NAVIGATE)
        .ok_or_else(|| std::io::Error::other("navigation request was not observed"))?;
    assert_eq!(navigation.args[0]["session"], "session-1");
    assert_eq!(navigation.args[0]["url"], "https://example.com");
    let held_output = seen
        .iter()
        .find(|call| call.member == tinycomputer_bus::browser::names::methods::READ_OUTPUT)
        .ok_or_else(|| std::io::Error::other("held-output request was not observed"))?;
    assert_eq!(held_output.args[0]["output"], "held-image-1");
    assert_eq!(held_output.args[0]["offset"], 0);
    assert_eq!(
        held_output.args[0]["max_len"],
        ReadOutputRequest::DEFAULT_LEN
    );

    assert_eq!(RUN_GOAL_TIMEOUT, Duration::from_secs(330));
    assert_eq!(TASK_CALL_TIMEOUT, Duration::from_secs(75));
    Ok(())
}
