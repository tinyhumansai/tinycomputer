//! Catalogue routing and confidentiality behavior.

use super::*;

#[tokio::test]
async fn every_catalogued_operation_is_routed_by_family_and_confidentiality()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = untrusted_fixture().await?;
    let client = client_for(&fixture.bus, None);
    let config = ComputerConfig::default();
    client.record_loaded_with(&config)?;
    let mut called = 0;
    let mut refused_confidential = 0;

    for entry in catalogue::MEMBERS {
        assert_eq!(
            entry.family,
            expected_family(entry.name),
            "family for {}",
            entry.name
        );
        assert_eq!(
            entry.confidential,
            handler_requires_confidential(entry.name),
            "confidential flag for {}",
            entry.name
        );
        let request = catalogue_request(entry.name)?;
        let result = if entry.name == task_names::methods::DESCRIBE {
            let described = client.describe(&config).await?;
            assert_eq!(
                described.contract_version,
                tinycomputer_bus::CONTRACT_VERSION
            );
            called += 1;
            continue;
        } else {
            match entry.family {
                Family::Task => {
                    client
                        .call_task::<_, Value>(&config, entry.name, request)
                        .await
                }
                Family::Flow | Family::Desktop | Family::Browser => {
                    client
                        .call_desktop::<_, Value>(&config, entry.name, request)
                        .await
                }
            }
        };

        if entry.confidential {
            assert!(
                matches!(result, Err(ClientError::Bus(_))),
                "unattested recipient must refuse confidential member {}",
                entry.name
            );
            refused_confidential += 1;
        } else {
            let value = result?;
            assert_catalogue_reply(entry.name, &value);
            called += 1;
        }
    }

    assert!(called > 0);
    assert!(refused_confidential > 0);
    assert_eq!(
        fixture.calls.load(std::sync::atomic::Ordering::Acquire),
        called,
        "only public calls and Describe reach an untrusted service"
    );
    let seen = fixture
        .seen
        .lock()
        .map_err(|_| std::io::Error::other("untrusted call log poisoned"))?
        .clone();
    assert!(seen.iter().all(|call| !call.confidential));
    Ok(())
}
