# Browser egress proxy

This module gives an explicitly launched Chromium session a private HTTP proxy
whose destination decisions come from the TinyComputer host callback. It keeps
the engine adapter (`EgressAuthorizer`) small: the host authorizes the parsed
destination, returns a one-use grant and exact socket addresses, consumes a
grant before each dial, and revalidates active leases.

## Structure

- `mod.rs` defines the public `BrowserDialTarget`, `DialGrant`,
  `EgressAuthorizer`, and `EgressProxy` surface and owns the loopback listener.
- `request.rs` authenticates each local proxy client, parses HTTP proxy
  requests, validates authorities and framing, and streams fixed-length and
  chunked request bodies with bounded buffers.
- `forward.rs` writes sanitized upstream headers and forwards request and
  response data while checking active leases.
- `serve.rs` sequences authorization, grant consumption, exact-address
  connection, request forwarding, and response handling for one connection.

## Operational constraints

The proxy binds IPv4 loopback and requires Basic authentication with a random
per-session password. It never resolves destination names; it dials only an
address returned by the host and consumes the matching one-use grant first.
Only absolute `http` proxy requests, CONNECT requests, WebSocket upgrades,
fixed-length uploads, and valid HTTP/1.1 chunked uploads are accepted. Ambiguous
framing, caller-selected transfer encodings, unauthenticated clients, malformed
chunk framing, and bytes following a framed request body are refused.

HTTPS payloads remain opaque after CONNECT. The proxy checks the lease at least
once per second while uploading, tunneling, or receiving a response, and closes
the connection when validation is denied or fails. Do not add a direct-dial
fallback or locally resolve a host here: destination policy belongs to the
authenticated host callback.
