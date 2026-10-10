# Native Globe resource ownership

The module owns the existing Swift listener process. `owned.rs` retains its native
process group and reader workers until termination, reaping and joining succeed.
A cleanup failure keeps the resource reachable; a reader panic is reported after
all readers complete and is safe to retry. `compiler.rs` retains partial compiler
ownership with the same rules, fixed-buffer output draining, terminal cancellation
and a 30-second bound. `helper.rs` preserves private cache layout and Swift source.

`queue.rs` retains at most 64 physical FN_DOWN/FN_UP facts in order and reports
loss. `reader.rs` bounds each stdout frame to 128 bytes, discards stderr in 8 KiB
chunks and never logs native payloads. Native restart marks a continuity gap.
Any unexpected stdout reader termination also marks a gap, including clean EOF
while the helper process remains alive; explicit Stop discards its queue.
The bus module retains one acknowledged snapshot above this queue and exposes
explicit reset semantics; legacy Poll keeps its old destructive wire.

Physical macOS permission/hook behavior stays here. Other platforms retain their
unsupported status without opening devices. Tests use local mock subprocesses,
controlled pipe readers and native cleanup faults; no physical hooks are opened.
Cross-target checks compile macOS wiring. Module owners must successfully call
terminal GlobeShutdown before ABI unload; Drop is a fallback, not an acknowledgment.
