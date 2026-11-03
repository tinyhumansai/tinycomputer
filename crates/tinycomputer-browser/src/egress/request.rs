//! Parses and validates browser proxy requests before a destination is authorized.

use std::{io, time::Duration};

mod body;
pub(super) use body::{buffered_body_len, has_valid_buffered_body, read_request_body};

use base64::Engine as _;
use tokio::{io::AsyncReadExt, net::TcpStream};

use super::{
    BrowserDialTarget,
    forward::{constant_time_eq, refuse, write_all_bounded},
};

pub(super) struct ProxyRequest {
    pub(super) bytes: Vec<u8>,
    pub(super) split: usize,
    pub(super) lines: Vec<String>,
    pub(super) method: String,
    pub(super) uri: String,
    pub(super) version: String,
    pub(super) target: BrowserDialTarget,
    pub(super) connect: bool,
    pub(super) body_len: usize,
    pub(super) chunked: bool,
    pub(super) has_upgrade: bool,
}

pub(super) async fn read_proxy_request(
    client: &mut TcpStream,
    session_id: &str,
    proxy_password: &str,
) -> io::Result<Option<ProxyRequest>> {
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(10), client.read(&mut chunk))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "proxy request timed out"))??;
        if n == 0 {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > 32 * 1024 {
            refuse(client).await?;
            return Ok(None);
        }
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        refuse(client).await?;
        return Ok(None);
    };
    let split = header_end + 4;
    let Ok(head) = std::str::from_utf8(&bytes[..split]) else {
        refuse(client).await?;
        return Ok(None);
    };
    let lines: Vec<String> = head.split("\r\n").map(str::to_owned).collect();
    let Some(request_line) = lines.first() else {
        refuse(client).await?;
        return Ok(None);
    };
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let uri = parts.next().unwrap_or_default().to_owned();
    let version = parts.next().unwrap_or_default().to_owned();
    if parts.next().is_some() || (version != "HTTP/1.1" && version != "HTTP/1.0") {
        refuse(client).await?;
        return Ok(None);
    }
    let Some(target) = proxy_target(&method, &uri, session_id) else {
        refuse(client).await?;
        return Ok(None);
    };
    let connect = method.eq_ignore_ascii_case("CONNECT");
    let Some(headers) = parse_proxy_headers(&lines, &target, connect, session_id) else {
        refuse(client).await?;
        return Ok(None);
    };
    if !has_proxy_auth(headers.proxy_authorization.as_deref(), proxy_password) {
        write_all_bounded(
            client,
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"tinycomputer-browser\"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            "browser proxy challenge timed out",
        )
        .await?;
        return Ok(None);
    }
    if connect && bytes.len() > split {
        refuse(client).await?;
        return Ok(None);
    }
    Ok(Some(ProxyRequest {
        bytes,
        split,
        lines,
        method,
        uri,
        version,
        target,
        connect,
        body_len: headers.body_len,
        chunked: headers.chunked,
        has_upgrade: headers.has_upgrade,
    }))
}

fn proxy_target(method: &str, uri: &str, session_id: &str) -> Option<BrowserDialTarget> {
    if method.eq_ignore_ascii_case("CONNECT") {
        return parse_authority(uri, "https", session_id).ok();
    }
    let url = url::Url::parse(uri).ok()?;
    if !matches!(url.scheme(), "http" | "ws")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(BrowserDialTarget {
        scheme: "http".to_owned(),
        hostname: url
            .host_str()?
            .trim_matches(['[', ']'])
            .to_ascii_lowercase(),
        port: url.port_or_known_default()?,
        session_id: session_id.to_owned(),
    })
}

struct ParsedHeaders {
    body_len: usize,
    chunked: bool,
    has_upgrade: bool,
    proxy_authorization: Option<String>,
}

fn parse_proxy_headers(
    lines: &[String],
    target: &BrowserDialTarget,
    connect: bool,
    session_id: &str,
) -> Option<ParsedHeaders> {
    let mut body_len = 0usize;
    let mut host_header = None;
    let mut has_upgrade = false;
    let mut connection_upgrade = false;
    let mut proxy_authorization = None;
    let mut saw_content_length = false;
    let mut chunked = false;
    let mut saw_transfer_encoding = false;
    for line in lines
        .iter()
        .skip(1)
        .map(String::as_str)
        .filter(|line| !line.is_empty())
    {
        let (name, value) = line.split_once(':')?;
        // RFC 9112 forbids whitespace in or after a field name: a server
        // could read `Content-Length : 5` as a header this check skipped.
        if name.is_empty() || name.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return None;
        }
        if name.eq_ignore_ascii_case("host") {
            if host_header.replace(value.trim().to_owned()).is_some() {
                return None;
            }
        } else if name.eq_ignore_ascii_case("content-length") {
            if saw_content_length {
                return None;
            }
            saw_content_length = true;
            body_len = value.trim().parse().ok()?;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            if saw_transfer_encoding || !value.trim().eq_ignore_ascii_case("chunked") {
                return None;
            }
            saw_transfer_encoding = true;
            chunked = true;
        } else if name.eq_ignore_ascii_case("expect") {
            return None;
        } else if name.eq_ignore_ascii_case("proxy-authorization") {
            if proxy_authorization
                .replace(value.trim().to_owned())
                .is_some()
            {
                return None;
            }
        } else if name.eq_ignore_ascii_case("upgrade")
            && value.trim().eq_ignore_ascii_case("websocket")
        {
            has_upgrade = true;
        } else if name.eq_ignore_ascii_case("connection")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        {
            connection_upgrade = true;
        }
    }
    let host_matches = host_header.as_deref().is_some_and(|host| {
        parse_authority(host, if connect { "https" } else { "http" }, session_id)
            .is_ok_and(|parsed| parsed.hostname == target.hostname && parsed.port == target.port)
    });
    if has_upgrade != connection_upgrade
        || !host_matches
        || (saw_content_length && chunked)
        || ((connect || has_upgrade) && (body_len != 0 || chunked))
    {
        return None;
    }
    Some(ParsedHeaders {
        body_len,
        chunked,
        has_upgrade,
        proxy_authorization,
    })
}

fn has_proxy_auth(proxy_authorization: Option<&str>, password: &str) -> bool {
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("tinycomputer:{password}"))
    );
    proxy_authorization
        .is_some_and(|actual| constant_time_eq(actual.as_bytes(), expected.as_bytes()))
}
pub(crate) fn parse_authority(
    value: &str,
    scheme: &str,
    session_id: &str,
) -> io::Result<BrowserDialTarget> {
    if value.contains("://") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "authority form required",
        ));
    }
    let authority = url::Url::parse(&format!("{scheme}://{value}/"))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid CONNECT authority"))?;
    if authority.path() != "/"
        || authority.query().is_some()
        || authority.fragment().is_some()
        || !authority.username().is_empty()
        || authority.password().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid CONNECT authority",
        ));
    }
    Ok(BrowserDialTarget {
        scheme: scheme.to_owned(),
        hostname: authority
            .host_str()
            .unwrap_or_default()
            .trim_matches(['[', ']'])
            .to_ascii_lowercase(),
        port: authority
            .port()
            .unwrap_or(if scheme == "http" { 80 } else { 443 }),
        session_id: session_id.to_owned(),
    })
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
