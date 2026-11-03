# Trace sink adapter

This module connects the desktop engine's synchronous trace-sink interface to
the host's asynchronous TinyBus `TraceSinkHost` callbacks. `TraceLink` owns a
single named worker thread and exposes the engine adapter in
`engine_adapter.rs`; the parent module constructs it only when configuration
contains the unique callback destination prepared by the host.

## Callback flow

`append` and `store_artifact` copy the callback data into a bounded, one-slot
queue. The worker serializes accepted calls and performs the TinyBus request
using the runtime handle captured when `TraceLink` was created. A synchronous
engine callback waits only until its supplied deadline, capped by
`CALL_TIMEOUT`. Events carry their remaining deadline and byte budget; artifact
requests also carry their size limits. Artifacts larger than the bus contract's
8 MiB decoded limit are rejected before queueing. Production callbacks use
confidential TinyBus calls so trace contents and artifacts are not exposed as
ordinary module traffic.

Task reports use `write_report` on the async dispatch path and are sent through
the same host interface. The callback-active guard rejects overlapping calls
across both paths, providing bounded backpressure and preventing recursive
host callbacks from waiting on one another. It detects overlap, not causal
re-entrancy: the host must not call back into this module and wait for another
trace callback to finish.

## Lifecycle and constraints

Construction requires a Tokio multi-thread runtime because the worker drives
async bus calls while the engine may synchronously wait for their result. The
worker queue has capacity one; a full queue fails the engine callback promptly
instead of growing memory or blocking the caller. The bus proxy timeout uses
the remaining callback deadline, so an expired operation is not sent.

`shutdown` closes the queue, joins the worker, and waits briefly for an
in-flight report callback before the service is torn down. `Drop` invokes the
same shutdown path as a best effort. A shutdown error is surfaced during
explicit service teardown; the destructor cannot report it. Tests under
`trace_sink_tests/` cover wire payloads, callback ordering and errors, deadlines,
and worker shutdown over the in-memory bus.
