# tinycomputer-browser

Part of [tinycomputer](../../README.md), a decision model (Jev) based harness for
desktop and browser automation, written in Rust. This crate is its browser side: it drives Chrome and shows the harness a web page. Its user guide is
[`docs/crates/tinycomputer-browser/`](../../docs/crates/tinycomputer-browser/README.md).

The agent-browser adapter behind tinycomputer's `Browser` interface. It turns the
typed requests in `tinycomputer_bus::browser` into agent-browser commands and
the engine's replies into typed results. agent-browser is linked in-process as
a library; there is no daemon and no sidecar.

The crate has no bus, no agent loop and no model. `tinycomputer-engine` drives
it as a surface next to the desktop, and `tinycomputer` serves it over TinyBus.

| Path | Holds |
|---|---|
| `src/sessions/` | `Browser`: up to eight sessions, each with its own engine and scratch directory; calls on one session are serialized |
| `src/engine/` | the `Engine` and `Launcher` seam: one JSON command in, one reply out, so everything above it is tested with a scripted engine |
| `src/linked/` | `AgentBrowser` (feature `agent-browser`): agent-browser's dispatcher linked in-process, one `DaemonState` per session, built without reading the host's `AGENT_BROWSER_*` environment |
| `src/convert/` | contract requests to agent-browser commands, as pure functions |
| `src/reply/` | agent-browser replies to typed results |
| `src/deep/` | controls a page keeps inside open shadow roots that the accessibility tree gives no ref: looked for after every snapshot (`deep.js`), listed with refs `d1`, `d2`, …, and pressed, typed into, or read where they show (`src/sessions/deep.rs`) |
| `src/surface/` | `BrowserSurface`: one session as a `tinycomputer-core` `Surface`; `sight/`, which reads the rendered page the way a person looks at it (the default, `docs/technical/specs/browser-sight.md`), and `tree.rs`, which parses snapshot text into a `Screen` when sight cannot reach the page or `Perception::Tree` is chosen |
| `src/error/` | `Error`: what a caller should do next, one published wire name per variant |
| `src/outputs/` | held screenshots and PDFs: bounded count, size and lifetime, chunked reads |
| `src/fake/` | the scripted engine tests use |

`BrowserSurface` opens its session lazily and can attach to a running Chrome
through `SessionOptions::endpoint` instead of launching one. Closing an
attached session only disconnects. See `docs/technical/architecture.md` for how the
surface compares with the desktop's.

When the compiled TinyComputer module receives a host-private `browser.egress`
scope, it selects `EnforcedAgentBrowser`: every session must be explicitly
launched by TinyComputer, attaches and caller launch overrides are refused,
and the process uses an authenticated per-session loopback proxy. The proxy
asks the host callback for TinySecurity's one-use, generation-bound dial
authorization and dials only the returned socket address. It periodically
validates active leases, including TLS CONNECT and WebSocket streams. HTTPS
contents remain opaque to the proxy and require TinySecurity's explicit
`OpaqueBrowserTraffic` classification. See
[`configuration.md`](../../docs/crates/tinycomputer/configuration.md#browser)
for the host contract and its privacy behavior.

The proxy internals and request-framing constraints are described in
[`src/egress/README.md`](src/egress/README.md).

The Chromium egress fixture tests are excluded from the default suite because
they need an installed browser; run them with
`TINYCOMPUTER_LIVE_BROWSER_TESTS=1 cargo test -p tinycomputer-browser --features agent-browser live_`.
They use local fixtures only. Other browser integration tests run in the Docker
lab (`docs/technical/docker-lab.md`).
