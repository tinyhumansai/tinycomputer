//! Forwards authorized proxy traffic and keeps active streams tied to their lease.

use std::{io, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::TcpStream,
};

use super::{
    EgressAuthorizer,
    request::{ProxyRequest, buffered_body_len, read_request_body},
};

pub(super) async fn initialize_upstream(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    request: &ProxyRequest,
    auth: &Arc<dyn EgressAuthorizer>,
    lease: &str,
    validations: &LeaseValidationClock,
) -> io::Result<bool> {
    if request.connect {
        write_all_bounded(
            client,
            b"HTTP/1.1 200 Connection Established\r\n\r\n",
            "browser CONNECT response timed out",
        )
        .await?;
        if request.bytes.len() > request.split {
            return write_all_with_lease(
                upstream,
                &request.bytes[request.split..],
                "browser CONNECT payload timed out",
                auth,
                lease,
                validations,
            )
            .await;
        }
        return Ok(true);
    }
    let url = url::Url::parse(&request.uri)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP proxy URL"))?;
    let path = if let Some(query) = url.query() {
        format!("{}?{query}", url.path())
    } else {
        url.path().to_owned()
    };
    let mut forwarded = format!("{} {path} {}\r\n", request.method, request.version);
    let headers = || {
        request
            .lines
            .iter()
            .skip(1)
            .filter(|line| !line.is_empty())
            .map(|line| (line, line.split_once(':').map_or("", |(name, _)| name)))
    };
    // RFC 9110 §7.6.1: the fields `Connection` nominates are this hop's only.
    // `Upgrade` stays for a websocket request, and the framing fields the
    // proxy read the body by stay so the upstream frames it the same way.
    let nominated: Vec<String> = headers()
        .filter(|(_, name)| {
            name.eq_ignore_ascii_case("connection") || name.eq_ignore_ascii_case("proxy-connection")
        })
        .flat_map(|(line, name)| line[name.len() + 1..].split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !["upgrade", "host", "content-length", "transfer-encoding"].contains(&token.as_str()))
        .collect();
    for (line, name) in headers() {
        let name = name.to_ascii_lowercase();
        let hop_by_hop = matches!(
            name.as_str(),
            "connection" | "proxy-connection" | "proxy-authorization" | "keep-alive" | "te"
        ) || (name == "upgrade" && !request.has_upgrade)
            || nominated.contains(&name);
        if !hop_by_hop {
            forwarded.push_str(line);
            forwarded.push_str("\r\n");
        }
    }
    if request.has_upgrade {
        forwarded.push_str("Connection: Upgrade\r\n\r\n");
    } else {
        forwarded.push_str("Connection: close\r\n\r\n");
    }
    if !write_all_with_lease(
        upstream,
        forwarded.as_bytes(),
        "browser upstream request timed out",
        auth,
        lease,
        validations,
    )
    .await?
    {
        return Ok(false);
    }
    if request.chunked {
        Ok(true)
    } else {
        write_all_with_lease(
            upstream,
            &request.bytes[request.split..],
            "browser upstream body timed out",
            auth,
            lease,
            validations,
        )
        .await
    }
}
pub(super) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

pub(super) async fn tunnel_with_lease(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    auth: &Arc<dyn EgressAuthorizer>,
    lease: String,
    validations: &LeaseValidationClock,
) -> io::Result<()> {
    let relay = copy_bidirectional(client, upstream);
    tokio::pin!(relay);
    loop {
        tokio::select! {
            biased;
            () = wait_for_validation(validations) => {
                if !validate_lease(auth, &lease).await? { return Ok(()); }
            },
            result = &mut relay => { result?; return Ok(()); },
        }
    }
}

pub(super) async fn forward_response_with_lease<C, U>(
    client: &mut C,
    upstream: &mut U,
    auth: &Arc<dyn EgressAuthorizer>,
    lease: String,
    validations: &LeaseValidationClock,
) -> io::Result<()>
where
    C: tokio::io::AsyncWrite + Unpin,
    U: tokio::io::AsyncRead + Unpin,
{
    let mut chunk = [0u8; 8192];
    loop {
        tokio::select! {
            biased;
            () = wait_for_validation(validations) => {
                if !validate_lease(auth, &lease).await? { return Ok(()); }
            },
            result = upstream.read(&mut chunk) => {
                let n = result?;
                if n == 0 { return Ok(()); }
                let write = write_all_bounded(
                    client,
                    &chunk[..n],
                    "browser response write timed out",
                );
                tokio::pin!(write);
                loop {
                    tokio::select! {
                        biased;
                        () = wait_for_validation(validations) => {
                            if !validate_lease(auth, &lease).await? { return Ok(()); }
                        },
                        result = &mut write => { result?; break; },
                    }
                }
            },
        }
    }
}

pub(super) async fn forward_request_body_with_lease<C, U>(
    client: &mut C,
    upstream: &mut U,
    request: &ProxyRequest,
    auth: &Arc<dyn EgressAuthorizer>,
    lease: &str,
    validations: &LeaseValidationClock,
) -> io::Result<bool>
where
    C: tokio::io::AsyncRead + Unpin,
    U: tokio::io::AsyncWrite + Unpin,
{
    let body = read_request_body(client, upstream, request, buffered_body_len(request));
    tokio::pin!(body);
    loop {
        tokio::select! {
            biased;
            () = wait_for_validation(validations) => {
                if !validate_lease(auth, lease).await? { return Ok(false); }
            },
            result = &mut body => { result?; return Ok(true); },
        }
    }
}

pub(super) type LeaseValidationClock = Arc<tokio::sync::Mutex<tokio::time::Interval>>;

pub(super) fn validation_interval() -> LeaseValidationClock {
    let period = Duration::from_secs(1);
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    Arc::new(tokio::sync::Mutex::new(interval))
}

async fn wait_for_validation(validations: &LeaseValidationClock) {
    validations.lock().await.tick().await;
}

pub(super) async fn validate_lease(
    auth: &Arc<dyn EgressAuthorizer>,
    lease: &str,
) -> io::Result<bool> {
    tokio::time::timeout(Duration::from_secs(2), auth.validate(lease.to_owned()))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "browser lease validation timed out",
            )
        })?
}
pub(super) async fn refuse(client: &mut TcpStream) -> io::Result<()> {
    write_all_bounded(
        client,
        b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        "browser refusal response timed out",
    )
    .await
}

pub(super) async fn write_all_bounded<W: tokio::io::AsyncWrite + Unpin>(
    stream: &mut W,
    bytes: &[u8],
    message: &'static str,
) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(bytes))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, message))?
}

pub(super) async fn write_all_with_lease<W: tokio::io::AsyncWrite + Unpin>(
    stream: &mut W,
    bytes: &[u8],
    message: &'static str,
    auth: &Arc<dyn EgressAuthorizer>,
    lease: &str,
    validations: &LeaseValidationClock,
) -> io::Result<bool> {
    let write = write_all_bounded(stream, bytes, message);
    tokio::pin!(write);
    loop {
        tokio::select! {
            biased;
            () = wait_for_validation(validations) => {
                if !validate_lease(auth, lease).await? { return Ok(false); }
            },
            result = &mut write => { result?; return Ok(true); },
        }
    }
}
