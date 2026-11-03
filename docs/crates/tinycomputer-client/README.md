# tinycomputer-client

`tinycomputer-client` is the reusable host-side caller for the TinyComputer
TinyBus interface. It takes request and response types from
`tinycomputer-bus`, so OpenHuman and other hosts do not need to copy member
names, payload types, envelope decoding, or confidential-call selection.

## Calls and configuration

Construct a [`tinycomputer_client::ComputerClient`] around one
`tinybus::module::ReconfigurableModule`. Every call takes a typed
`tinycomputer_bus::config::ComputerConfig`; the client holds the coordinator's
exclusive guard until the reply is decoded. Concurrent calls therefore cannot
reconfigure the module between configuration selection and dispatch. A host
that has just loaded the module with a known configuration can call
`record_loaded_with` to seed the coordinator's fingerprint.

Use `call_desktop` for desktop, accessibility, flow and browser members, and
`call_task` for the typed task envelopes. `describe` is separate because the
contract returns raw `Capabilities` rather than an Agent envelope. Each call
requires a name in the contract catalogue; unknown names and families added by
a newer contract fail closed. The response is checked with
`DesktopResponse::into_data` or `AgentResponse::into_data` before it reaches
the host.

The catalogue controls confidential delivery. Requests for member names marked
confidential are sent with TinyBus' confidential call path, which requires an
attested recipient; other members use ordinary routing. `RunGoal` gets a
330-second transport deadline so its bounded execution can return its
structured result. Task calls use a 75-second deadline, above the module's
long-poll cap.

## Policy stays with the host

This crate does not decide which websites a browser may visit, choose Chrome
paths, grant task origins or constraints, or link the module's execution
engines. Hosts build those values and pass the resulting contract configuration
and request types. Browser profile claims and held-output handles are ordinary
typed contract calls; the client does not retain, transform, or release those
payloads behind the caller's back.
