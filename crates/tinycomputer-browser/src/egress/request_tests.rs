//! Tests for proxy request header parsing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn target() -> BrowserDialTarget {
    BrowserDialTarget {
        scheme: "http".to_owned(),
        hostname: "site.test".to_owned(),
        port: 80,
        session_id: "session".to_owned(),
    }
}

fn parse(extra: &str) -> Option<ParsedHeaders> {
    let lines = vec![
        "POST http://site.test/form HTTP/1.1".to_owned(),
        "Host: site.test".to_owned(),
        extra.to_owned(),
    ];
    parse_proxy_headers(&lines, &target(), false, "session")
}

#[test]
fn a_well_formed_header_is_read() {
    assert_eq!(parse("Content-Length: 5").expect("parsed").body_len, 5);
}

#[test]
fn whitespace_in_or_after_a_field_name_refuses_the_request() {
    for header in [
        "Content-Length : 5",
        "Content-Length\t: 5",
        "Proxy-Authorization : Basic x",
        " Host: site.test",
        ": no name",
    ] {
        assert!(parse(header).is_none(), "{header:?}");
    }
}
