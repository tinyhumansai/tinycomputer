# Specifications

Specifications define what the system must do before implementation details
take over. Create one for behavior that changes a public API, crosses module
boundaries, introduces a durable data format, or has meaningful operational
constraints.

Use a short kebab-case filename such as `retry-policy.md`. Each specification
should contain:

1. **Status and owner** — Draft, Accepted, Implemented, or Superseded.
2. **Problem** — the user or system need, without prescribing a solution.
3. **Goals and non-goals** — the exact boundary of the work.
4. **Proposed behavior** — public API, inputs, outputs, errors, and examples.
5. **Invariants and constraints** — properties every implementation must keep.
6. **Acceptance criteria** — externally observable pass/fail conditions.
7. **Open questions** — unresolved decisions that block acceptance.

After the specification is accepted, create a linked implementation plan in
[`../plans/`](../plans/README.md). Keep code snippets small enough to clarify
the contract; production code still belongs under `src/`.

See [`process-containment.md`](process-containment.md) for which spawned processes the host cannot contain.

See [`desktop-module-contract.md`](desktop-module-contract.md) for a complete
example, [`jev-intent-flows.md`](jev-intent-flows.md) for the flow runtime, and
[`unified-agent.md`](unified-agent.md) for browser and desktop behind one task API,
[`jev-briefing.md`](jev-briefing.md) for briefs, secrets, and votes,
[`jev-wide-turns.md`](jev-wide-turns.md) for the wide strategy,
[`browser-sight.md`](browser-sight.md) for how the browser reads a page,
[`flow-reflection.md`](flow-reflection.md) for checking what a `choose` left,
[`jev-deliberation.md`](jev-deliberation.md) for deciding on evidence and
undoing mistakes, [`task-rescue.md`](task-rescue.md) for rescuing a failed
step with a reasoning model, [`task-output.md`](task-output.md) for
remembering saved values and shaping a task's answer, and
[`virtual-cursor.md`](virtual-cursor.md) for the agent's drawn cursor, and
[`captcha-solver.md`](captcha-solver.md) for trying a captcha before asking a
person.
