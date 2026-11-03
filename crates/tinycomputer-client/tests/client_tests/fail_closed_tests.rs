//! Unknown members, wrong families, malformed envelopes, and an unknown module.

use super::*;

#[tokio::test]
async fn unknown_members_wrong_families_and_bad_envelopes_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = untrusted_fixture().await?;
    let client = client_for(&fixture.bus, None);
    let config = ComputerConfig::default();
    client.record_loaded_with(&config)?;

    assert!(matches!(
        client
            .call_desktop::<_, Value>(&config, "AccessibilityFutureMember", Value::Null)
            .await,
        Err(ClientError::UnknownMember(_))
    ));
    assert!(matches!(
        client
            .call_task::<_, Value>(
                &config,
                tinycomputer_bus::browser::names::methods::NAVIGATE,
                Value::Null,
            )
            .await,
        Err(ClientError::WrongFamily { .. })
    ));
    assert!(matches!(
        client
            .call_desktop::<_, Value>(&config, task_names::methods::PLAN_TASK, Value::Null,)
            .await,
        Err(ClientError::WrongFamily { .. })
    ));
    assert!(matches!(
        client
            .call_desktop::<_, Value>(
                &config,
                names::methods::PERMISSIONS,
                json!({"malformed_version": true}),
            )
            .await,
        Err(ClientError::DesktopResponse(
            tinycomputer_bus::DesktopResponseError::Version { .. }
        ))
    ));
    assert!(matches!(
        client
            .call_desktop::<_, String>(
                &config,
                names::methods::PERMISSIONS,
                json!({"wrong_data": true}),
            )
            .await,
        Err(ClientError::DesktopResponse(
            tinycomputer_bus::DesktopResponseError::Decode(_)
        ))
    ));
    assert!(matches!(
        client
            .call_task::<_, Value>(
                &config,
                task_names::methods::PLAN_TASK,
                json!({"malformed_version": true}),
            )
            .await,
        Err(ClientError::AgentResponse(
            tinycomputer_bus::agent::AgentResponseError::Version { .. }
        ))
    ));
    Ok(())
}

#[tokio::test]
async fn a_module_the_host_cannot_find_fails_with_a_module_error()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = untrusted_fixture().await?;
    let client = client_for_unknown_module(&fixture.bus);
    assert!(matches!(
        client.describe(&ComputerConfig::default()).await,
        Err(ClientError::Module(_))
    ));
    Ok(())
}
