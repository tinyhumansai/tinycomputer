# Native accessibility contract

Contract 2.12 serves ten native members to the compiled computer module. They return
the existing DesktopResponse envelope and leave every existing arity unchanged.

| Member | Arguments | Data |
| --- | --- | --- |
| AccessibilityPermissions | none | PermissionStatus |
| AccessibilityRequestPermission | PermissionKind | PermissionStatus |
| AccessibilityFocus | FocusQuery | FocusedTextContext |
| AccessibilityValidateTarget | FocusTarget | null |
| AccessibilityPaste | PasteRequest | null |
| GlobeStart | none | GlobeStarted |
| GlobePoll | GlobeHandle | GlobeHotkeyPollResult |
| GlobeRead | GlobeRead | GlobeBatch |
| GlobeShutdown | none | GlobeHotkeyStatus |
| GlobeStop | GlobeHandle | GlobeHotkeyStatus |

Focus, target validation and paste require confidential bus delivery. Paste
checks the captured application, role and bounds inside the module before
calling the native insertion implementation. Existing platform validation and
native helper deadlines remain in force. Blocking native work runs away from
the bus dispatch task; the native listener retains its own thread/process rules.

The module owns one Globe listener, with an opaque random lease. Start is
idempotent while it is owned; poll drains the existing bounded ordered queue.
Stop releases the lease, and subsequent poll/stop calls return UNKNOWN_LISTENER.
Native Stop waits for process-group reaping and both pipe readers before releasing
ownership. A cleanup error retains the lease for retry. Start is still recoverable
by repeating it while active. A canceled caller cannot abandon pending native work.

Reliable clients use GlobeRead with `acknowledged_batch: null` initially and then
acknowledge each fully processed batch. An unacknowledged snapshot replays exactly;
repeated acknowledgments replay the next snapshot. Each contains at most 64 typed
FN_DOWN/FN_UP events. An overflow requires consumers to reset activation inactive
and await a physical release before rearming. Mixing legacy destructive GlobePoll
with reliable reads marks the next new batch as overflow; use Read exclusively for
reliable composition. Retired handles never receive successor events.

GlobeShutdown closes admission, cancels pending native compiler work and waits
joined cleanup. It is terminal and must succeed before ABI unload. Use per-handle
Stop for sign-out/reconnect while reusing the module. Dropping the final module
owner attempts the same cleanup, but cannot replace this fallible unload barrier.
Permission/approval decisions stay with the host and computer module; Voice
consumes authorized typed facts without linking this native implementation.

Permission kinds/states and Globe payload spellings are unchanged. Focus DTOs
are now serializable. The compatibility accessibility library re-exports these
definitions. This crate contains no native frameworks, devices, subprocesses,
transport, runtime, or listener implementation.

Hosts must pin a published module with contract 2.12 and verified artifact
digests before switching callers; contract 2.11 does not serve these members.
