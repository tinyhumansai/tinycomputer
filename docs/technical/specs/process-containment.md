# Process containment and egress

Status: Accepted. Owner: tinycomputer maintainers.
Part of tinyhumansai/openhuman#7328 (phase 5, sandbox coverage).

## Problem

A host that sandboxes spawned processes (OpenHuman's tinysecurity and tinybox
work) needs to know which processes this module starts, what network it can
reach, and which of those the host can contain. A process that is silently
skipped looks contained when it is not.

## Decision

tinycomputer is a native module loaded into the host process. It drives the
user's real desktop and a real Chrome, so it cannot run inside a jail that
hides those. The host treats the module as unsandboxed-by-design and gates it
at the member level (approval, payment stop, `POLICY_DENIED`). For browser
sessions, the host may also provide the module's `BrowserEgressHost` contract:
the module routes HTTP, CONNECT, and WebSocket traffic through a per-session
authenticated proxy, and the host authorizes each dial and supplies the exact
socket address. This constrains only browser traffic that uses the configured
proxy. It does not contain the module process, prevent other direct sockets,
or inspect HTTPS contents; the host still needs OS-level process isolation for
those guarantees.

## Process inventory

`crates/tinycomputer-core/tests/process_inventory.rs` fails if a crate under
`crates/` spawns a process from a file that is not named here.

| Process | Spawned from | Contract |
| --- | --- | --- |
| Chrome (browser sessions) | agent-browser (`vendor/agent-browser`, `native/cdp/chrome.rs`), started by an explicit `launch` | Runs as the user. When the host provides `BrowserEgressHost`, the module sets an authenticated per-session proxy and all browser dials pass through host authorization and address consumption. Without that host contract, normal agent-browser egress behavior applies. This is not process containment. Xvfb and certutil helpers are agent-browser's. |
| Cursor overlay helper (`tinycomputer-cursor-overlay`) | `crates/tinycomputer-cursor/src/screen/process.rs` | Unsupported. Draws on screen, reads commands on stdin, opens no network. Optional; absent means no drawn cursor. |
| Accessibility helper (Swift, macOS) | `crates/tinycomputer-accessibility/src/helper/process.rs`, `globe.rs`, `build.rs` | Unsupported. Needs the macOS accessibility grant of the host app, so it cannot be jailed. Opens no network. |
| `osascript`, `open`, `cmd`, `xcrun`, `swiftc` | `crates/tinycomputer-accessibility/src/{focus,paste,permissions}.rs`, `helper/process.rs` | Unsupported. Fixed argv, no model-supplied command line. Local desktop actions only. |
| agent-browser plugins, Lightpanda, Safari/Appium drivers, upgrade | vendor/agent-browser | Not used by this module's launch path; owned by agent-browser. |

## Egress

The module's own outbound traffic is the decision model, planner, rescue and
output model calls (the `jev`, `planner` config routes), made from the host
process. Browser traffic goes through the host-provided proxy when the
`BrowserEgressHost` is configured; the host authorizes every destination and
provides the socket address used for the dial. The module does not proxy its
own model calls, enforce policy on other sockets opened by the process, or
inspect encrypted browser payloads. `origins` checks classify pages; they do
not block network access.

## Revisit when

The remaining containment gaps are closed only when the host supplies
OS-level process isolation that the module can verify. Keep the browser egress
contract and tests aligned with `BrowserEgressHost` in `tinycomputer-bus`.
