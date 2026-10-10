# tinycomputer-accessibility

Part of [tinycomputer](../../README.md), a decision model (Jev) based harness for
desktop and browser automation, written in Rust. Its user guide is
[`docs/crates/tinycomputer-accessibility/`](../../docs/crates/tinycomputer-accessibility/README.md).

Desktop accessibility middleware for a host that needs answers **in-process and
synchronously**: which text field has focus, may this process see input events,
did the Globe key go down. It is a plain library with no TinyBus, no async
runtime, and no engine. It is not the agent-facing desktop surface: that is
[`tinycomputer-desktop`](../tinycomputer-desktop/README.md), served over the bus
by the `tinycomputer` module.

| Module | Holds |
|---|---|
| `focus` | `focused_text_context[_verbose]` and `validate_focused_target`: query and re-check the focused text element |
| `permissions` | Accessibility, Input Monitoring and Microphone detection and requests; macOS `ApplicationServices` / `CoreFoundation` / `IOKit` FFI |
| `globe` | The macOS Globe/Fn key listener (a Swift process reporting `FN_DOWN` / `FN_UP`) |
| `helper/` | The persistent Swift helper process (focus, paste, overlay over stdin/stdout JSON) and its embedded Swift source |
| `automation_state` | Session flag for a denied "System Events" Apple Events grant |
| `terminal`, `text_util` | Terminal-window heuristics and AX string normalisation |
| `types` | `FocusedTextContext`, `ElementBounds`, `PermissionKind`, `PermissionState`, `PermissionStatus` |
| `error` | `Error` and the crate-wide `Result<T>` used by fallible public operations |

## Features

| Feature | Effect |
|---|---|
| `microphone-probe` | Probe whether an input device is present with `cpal`. Device enumeration does not confirm recording authorization, so macOS and Windows return `Unknown` until the host opens a capture stream. Off by default. |

## Platforms

Everything real happens on macOS. Elsewhere focus queries return
`Error::UnsupportedPlatform`, `validate_focused_target` passes, the Globe listener reports
`supported: false`, and Accessibility / Input Monitoring are `Unsupported`.

## Unsafe

The workspace forbids `unsafe`; this crate lowers it to `deny` and only
`src/permissions.rs` allows it, for the macOS permission FFI, each block with a
`// SAFETY:` comment. Dependencies are `serde`, `serde_json`, `log`, and
`thiserror` (plus `cpal` behind `microphone-probe`).

The macOS paths (FFI, helper process) cannot run in Linux CI: tests cover the
pure logic and the non-macOS fallbacks, and macOS code is checked with
`cargo check --target aarch64-apple-darwin`.

Shared permission, focus, Globe and error types are re-exported from
`tinycomputer-bus::accessibility`. Hosts using the compiled module take the
pure bus crate; native operations remain in this implementation.

Globe helpers use owned process groups and reader threads. Successful Stop joins
native cleanup; failed reaping retains ownership for public retry. Native event
framing is bounded to 128 bytes and stderr uses an 8 KiB discard buffer. Compilation
uses an owned group with a 30-second deadline and terminal cancellation predicate.
The module's GlobeShutdown is the required fallible barrier before unloading.
Compatibility Start/Poll/Stop library paths retain their platform/status wires;
module recovery/start paths pass the terminal predicate through native work.
