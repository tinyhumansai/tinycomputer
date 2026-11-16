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
- `upstream.rs` holds `UpstreamProxy`, the optional host-configured upstream
  HTTP proxy for a list of destination domains, and its `CONNECT` handshake.

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

## Upstream proxies

A host may route some destination domains (each entry matches itself and its
subdomains) through an upstream HTTP proxy, for sites that refuse the host's
own address (`EgressProxy::bind_with_upstreams`). Routing never weakens the
policy:

1. The destination is authorized exactly as for a direct dial. Its grant is
   consumed for one approved address (IPv4 first), and that address is the
   tunnel's target: the upstream proxy resolves nothing, so split DNS or a
   rebinding answer at the proxy cannot move the connection elsewhere.
2. The upstream proxy's host and port are authorized through the same host
   callback as any other dial (`scheme` `http`), and the proxy socket opens
   only at an address that grant names, after consuming it. Nothing is
   resolved locally and nothing is dialed outside a grant.
3. The proxy is asked for `CONNECT <approved address>` with the configured
   `Proxy-Authorization: Basic` credentials. The browser's `Host` header and
   TLS SNI travel inside the tunnel unchanged. Plain `http` and WebSocket
   requests to a routed host travel the same kind of tunnel (to the URL's
   port, 80 when it names none), with the usual
   origin-form request written into it, so no request reaches a routed host
   other than through a tunnel.
4. Every lease check validates both the destination lease and the proxy
   lease; either one failing closes the connection.

A refused upstream dial answers the browser `403`; an upstream proxy that
answers but will not open the tunnel (a `407` for bad credentials, a `502`,
a timeout) answers `502` and logs only the upstream status. Only `http://`
upstream proxies are supported. The credentials stay inside `UpstreamProxy`:
its `Debug` output, its errors, and the log lines never carry them.
