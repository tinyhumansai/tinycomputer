# Native accessibility contract

Contract 2.11 adds eight members to the compiled computer module. They return
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
| GlobeStop | GlobeHandle | GlobeHotkeyStatus |

Focus, target validation and paste require confidential bus delivery. Paste
checks the captured application, role and bounds inside the module before
calling the native insertion implementation. Existing platform validation and
native helper deadlines remain in force. Blocking native work runs away from
the bus dispatch task; the native listener retains its own thread/process rules.

The module owns one Globe listener, with an opaque random lease. Start is
idempotent while it is owned; poll drains the existing bounded ordered queue.
Stop releases the lease, and subsequent poll/stop calls return UNKNOWN_LISTENER.
Dropping the module's final owner stops an abandoned listener. Hosts stop it on
user/session shutdown, and retain authorization and approval policy.

Permission kinds/states and Globe payload spellings are unchanged. Focus DTOs
are now serializable. The compatibility accessibility library re-exports these
definitions. This crate contains no native frameworks, devices, subprocesses,
transport, runtime, or listener implementation.

Hosts must pin a published module with contract 2.11 and verified artifact
digests before switching callers; contract 2.10 does not serve these members.
