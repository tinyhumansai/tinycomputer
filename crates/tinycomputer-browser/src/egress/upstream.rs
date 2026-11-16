//! Routes chosen destinations through a host-configured upstream HTTP proxy.
//!
//! Some sites refuse a datacenter address outright. A host can name an
//! upstream proxy (a residential or mobile exit, say) for a list of domains;
//! a browser request to one of them still passes the full destination
//! authorization, and then reaches the destination through a `CONNECT`
//! tunnel on the upstream proxy instead of a direct dial. The upstream
//! proxy's own address is authorized, granted and consumed through the same
//! host callback as any other dial, so this adds no unchecked socket and no
//! local name resolution.
//!
//! The proxy URL carries credentials. They live only in
//! [`UpstreamProxy::authorization`], are written only into the `CONNECT`
//! request, and never appear in `Debug` output, logs or errors.

use std::{fmt, future::Future, io, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

use base64::Engine as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{BrowserDialTarget, DialGrant, EgressAuthorizer};

/// The longest upstream `CONNECT` response head accepted.
const MAX_RESPONSE_HEAD: usize = 8 * 1024;
/// How long the upstream proxy has to answer a `CONNECT`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// An upstream HTTP proxy and the destination domains routed through it.
#[derive(Clone, PartialEq, Eq)]
pub struct UpstreamProxy {
    domains: Vec<String>,
    host: String,
    port: u16,
    authorization: Option<String>,
}

impl fmt::Debug for UpstreamProxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UpstreamProxy([REDACTED])")
    }
}

/// Why an upstream proxy setting was refused. The message never repeats the
/// URL, since it may carry credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpstreamProxyError(&'static str);

impl fmt::Display for UpstreamProxyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for UpstreamProxyError {}

impl UpstreamProxyError {
    /// What was wrong, as a static message that never quotes the URL.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        self.0
    }
}

impl UpstreamProxy {
    /// Route `domains` (each matching itself and every subdomain) through the
    /// proxy at `url`, an `http://[user:password@]host[:port]` URL whose
    /// percent-encoded credentials are sent as `Proxy-Authorization: Basic`.
    ///
    /// # Errors
    ///
    /// Returns an error, which never quotes the URL, when the URL is not a
    /// plain `http` proxy URL or a domain is not a DNS name.
    pub fn new(url: &str, domains: &[String]) -> Result<Self, UpstreamProxyError> {
        let domains = normalize_domains(domains)?;
        let parsed = url::Url::parse(url)
            .map_err(|_| UpstreamProxyError("upstream proxy url is not a valid URL"))?;
        if parsed.scheme() != "http" {
            return Err(UpstreamProxyError(
                "upstream proxy url must be an http:// proxy URL",
            ));
        }
        if !matches!(parsed.path(), "" | "/")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(UpstreamProxyError(
                "upstream proxy url must name only credentials, a host and a port",
            ));
        }
        let host = match parsed.host() {
            Some(url::Host::Domain(domain)) => domain.to_ascii_lowercase(),
            Some(url::Host::Ipv4(ip)) => ip.to_string(),
            Some(url::Host::Ipv6(ip)) => ip.to_string(),
            None => return Err(UpstreamProxyError("upstream proxy url has no host")),
        };
        let port = parsed
            .port_or_known_default()
            .ok_or(UpstreamProxyError("upstream proxy url has no port"))?;
        let user = percent_decode(parsed.username())?;
        let password = percent_decode(parsed.password().unwrap_or_default())?;
        if user.contains(&b':') {
            return Err(UpstreamProxyError(
                "upstream proxy user name must not contain ':'",
            ));
        }
        let authorization = (!user.is_empty() || !password.is_empty()).then(|| {
            let mut credentials = user;
            credentials.push(b':');
            credentials.extend_from_slice(&password);
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(credentials)
            )
        });
        Ok(Self {
            domains,
            host,
            port,
            authorization,
        })
    }

    /// Whether a request for `hostname` goes through this proxy: `hostname`
    /// equals one of its domains or is a subdomain of one.
    #[must_use]
    pub fn routes(&self, hostname: &str) -> bool {
        let hostname = hostname.trim_end_matches('.').to_ascii_lowercase();
        self.domains.iter().any(|domain| {
            hostname == *domain
                || hostname
                    .strip_suffix(domain.as_str())
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
    }

    /// The dial the host authorizes for the proxy itself.
    pub(super) fn dial_target(&self, session_id: &str) -> BrowserDialTarget {
        BrowserDialTarget {
            scheme: "http".to_owned(),
            hostname: self.host.clone(),
            port: self.port,
            session_id: session_id.to_owned(),
        }
    }
}

fn normalize_domains(domains: &[String]) -> Result<Vec<String>, UpstreamProxyError> {
    if domains.is_empty() {
        return Err(UpstreamProxyError(
            "upstream proxy needs at least one domain",
        ));
    }
    domains
        .iter()
        .map(|domain| {
            let domain = domain.trim().trim_matches('.').to_ascii_lowercase();
            let valid = !domain.is_empty()
                && domain.len() <= 253
                && domain.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                });
            if valid {
                Ok(domain)
            } else {
                Err(UpstreamProxyError(
                    "upstream proxy domain must be a DNS name such as example.com",
                ))
            }
        })
        .collect()
}

fn percent_decode(value: &str) -> Result<Vec<u8>, UpstreamProxyError> {
    let invalid =
        UpstreamProxyError("upstream proxy url has malformed percent-encoded credentials");
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3).ok_or(invalid)?;
            let hex = std::str::from_utf8(hex).map_err(|_| invalid)?;
            decoded.push(u8::from_str_radix(hex, 16).map_err(|_| invalid)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Ok(decoded)
}

/// Ask the upstream proxy on `stream` for a tunnel to `destination`, an
/// address the host approved, so the proxy resolves nothing. On success the
/// stream carries bytes to and from the destination, exactly as a direct
/// socket to it would.
///
/// # Errors
///
/// Returns an error naming only the upstream status when the proxy refuses
/// (`PermissionDenied` for 407, `ConnectionRefused` otherwise), or an I/O,
/// timeout or framing error. No error carries the credentials.
pub(super) async fn open_tunnel<S>(
    stream: &mut S,
    proxy: &UpstreamProxy,
    destination: SocketAddr,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // `SocketAddr` displays an IPv6 address in brackets, as an authority.
    let authority = destination.to_string();
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(authorization) = &proxy.authorization {
        request.push_str("Proxy-Authorization: ");
        request.push_str(authorization);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        stream.write_all(request.as_bytes()).await?;
        drop(request);
        let head = read_response_head(stream).await?;
        let status = response_status(&head)?;
        if (200..300).contains(&status) {
            Ok(())
        } else if status == 407 {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "upstream proxy refused the credentials (407)",
            ))
        } else {
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("upstream proxy refused the tunnel ({status})"),
            ))
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "upstream proxy did not answer"))?
}

/// Read the response head one byte at a time, so nothing the destination
/// sends after it is consumed here.
async fn read_response_head<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_RESPONSE_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "upstream proxy response head is too long",
            ));
        }
        if stream.read(&mut byte).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "upstream proxy closed before answering",
            ));
        }
        head.push(byte[0]);
    }
    Ok(head)
}

fn response_status(head: &[u8]) -> io::Result<u16> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "upstream proxy sent no HTTP status",
        )
    };
    let line = head
        .split(|byte| *byte == b'\r')
        .next()
        .ok_or_else(invalid)?;
    let line = std::str::from_utf8(line).map_err(|_| invalid())?;
    let mut parts = line.split_ascii_whitespace();
    if !parts
        .next()
        .is_some_and(|version| version.starts_with("HTTP/1."))
    {
        return Err(invalid());
    }
    parts
        .next()
        .filter(|code| code.len() == 3)
        .and_then(|code| code.parse().ok())
        .ok_or_else(invalid)
}

/// The authorizer a routed connection forwards under: every lease check
/// requires both the destination's lease and the upstream proxy's, so a
/// revoked destination closes the tunnel just as a revoked proxy dial does.
pub(super) struct RoutedLease {
    pub(super) inner: Arc<dyn EgressAuthorizer>,
    pub(super) destination_lease: String,
}

impl EgressAuthorizer for RoutedLease {
    fn authorize(
        &self,
        target: BrowserDialTarget,
    ) -> Pin<Box<dyn Future<Output = io::Result<DialGrant>> + Send>> {
        self.inner.authorize(target)
    }

    fn consume(
        &self,
        target: BrowserDialTarget,
        grant: DialGrant,
        address: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send>> {
        self.inner.consume(target, grant, address)
    }

    fn validate(&self, lease: String) -> Pin<Box<dyn Future<Output = io::Result<bool>> + Send>> {
        let inner = self.inner.clone();
        let destination = self.destination_lease.clone();
        Box::pin(
            async move { Ok(inner.validate(destination).await? && inner.validate(lease).await?) },
        )
    }
}

#[cfg(test)]
#[path = "upstream_tests.rs"]
mod tests;
