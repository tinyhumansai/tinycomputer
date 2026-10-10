//! The pages a session may show: `SessionOptions::allowed_origins`, read once
//! when the session opens and asked of every page it navigates to.
//!
//! Only pages are checked: a navigation before it is sent, and the page every
//! call leaves the session on. The files a page loads are never checked. A
//! site draws its pages from its own CDN and calls APIs on other hosts, and
//! refusing those breaks the page (pictures and scripts missing, suggestion
//! lists empty) without keeping the agent anywhere it could not already go.
//! The list is a guard rail, not a sandbox.
//!
//! An address is read by the WHATWG URL rules a browser reads it by, so one
//! written another way (`http://2130706433/`, `http:\\host\`, a host in
//! percent escapes or full-width digits) is judged as the page it opens. A
//! name is never resolved: `*` refuses the addresses and names that are local
//! by how they are written, but a public name that resolves to a local
//! address (a wildcard DNS service, or DNS rebinding) is admitted.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Origin, Url};

/// What a session's `allowed_origins` admits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Origins {
    /// Whether a list was given: a list admits only what one of its entries
    /// names, even when no entry could be read.
    restricted: bool,
    entries: Vec<Entry>,
}

/// One entry of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// `*`: any public host. Private, loopback, link-local, and other
    /// non-global addresses, and local names ([`local_name`]), stay refused.
    Public,
    /// `https://example.com`, `example.com`, or an address: that host alone.
    Host(Host),
    /// `.example.com`, `https://.example.com`, or `*.example.com`: the name
    /// and every subdomain of it.
    Domain(String),
}

impl Origins {
    /// The origins `list` names, spelled as `SessionOptions::allowed_origins`
    /// spells them. An entry that names no host is skipped.
    pub(crate) fn new(list: &[String]) -> Self {
        Self {
            restricted: !list.is_empty(),
            entries: list
                .iter()
                .filter_map(|origin| Entry::parse(origin))
                .collect(),
        }
    }

    /// Whether a list was given, so pages are checked at all.
    pub(crate) fn restricts(&self) -> bool {
        self.restricted
    }

    /// Whether a page at `url` may show. Any page may when no list was given.
    /// Under a list, a blank page (`about:blank`, `about:srcdoc`) and a
    /// browser error page may, a web page may when an entry names its host,
    /// a `blob:` page when an entry names the host that made it, and any
    /// other address (a file, `data:`, a browser setting, `about:settings`,
    /// which the browser reads as one) may not. An address with no scheme is
    /// read as `https://`.
    pub(crate) fn admits(&self, url: &str) -> bool {
        if !self.restricted {
            return true;
        }
        let Some(page) = page_url(url) else {
            return false;
        };
        let host = match page.scheme() {
            "about" => return matches!(page.path(), "blank" | "srcdoc"),
            "chrome-error" => return true,
            "http" | "https" | "ws" | "wss" => page.host().map(|host| canonical(&host)),
            "blob" => match page.origin() {
                Origin::Tuple(_, host, _) => Some(canonical(&host)),
                Origin::Opaque(_) => None,
            },
            _ => None,
        };
        host.is_some_and(|host| self.entries.iter().any(|entry| entry.admits(&host)))
    }
}

impl Entry {
    /// The entry `origin` spells, or `None` when it names no host.
    fn parse(origin: &str) -> Option<Self> {
        let origin = origin.trim();
        if origin == "*" {
            return Some(Self::Public);
        }
        let rest = origin.split_once("://").map_or(origin, |(_, rest)| rest);
        let authority = rest
            .trim_start_matches('/')
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default();
        let authority = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let (subdomains, host) = match authority
            .strip_prefix("*.")
            .or_else(|| authority.strip_prefix('.'))
        {
            Some(domain) => (true, domain),
            None => (false, authority),
        };
        // The host without its port; an IPv6 address keeps its brackets.
        let host = match host.find(']') {
            Some(end) if host.starts_with('[') => &host[..=end],
            _ => host.split(':').next().unwrap_or_default(),
        };
        if host.is_empty() {
            return None;
        }
        match canonical(&Host::parse(host).ok()?) {
            Host::Domain(name) if name.is_empty() => None,
            Host::Domain(name) if subdomains => Some(Self::Domain(name)),
            host => Some(Self::Host(host)),
        }
    }

    /// Whether this entry names `host`, a host as [`canonical`] gives it.
    fn admits(&self, host: &Host) -> bool {
        match self {
            Self::Public => match host {
                Host::Domain(name) => !local_name(name),
                Host::Ipv4(address) => !non_global_v4(*address),
                Host::Ipv6(address) => !non_global_v6(*address),
            },
            Self::Host(named) => host == named,
            Self::Domain(domain) => match host {
                Host::Domain(name) => {
                    name == domain
                        || name
                            .strip_suffix(domain.as_str())
                            .is_some_and(|subdomain| subdomain.ends_with('.'))
                }
                Host::Ipv4(_) | Host::Ipv6(_) => false,
            },
        }
    }
}

/// `url` as the browser reads it, or `None` when it reads no address. An
/// address with no scheme (`example.com/flights`) is read as `https://`.
fn page_url(url: &str) -> Option<Url> {
    let url = url.trim();
    match Url::parse(url) {
        Err(url::ParseError::RelativeUrlWithoutBase) => Url::parse(&format!("https://{url}")).ok(),
        parsed => parsed.ok(),
    }
}

/// `host` as entries compare it: a name without its trailing dot, an
/// address as itself.
fn canonical<S: AsRef<str>>(host: &Host<S>) -> Host {
    match host {
        Host::Domain(name) => Host::Domain(name.as_ref().trim_end_matches('.').to_owned()),
        Host::Ipv4(address) => Host::Ipv4(*address),
        Host::Ipv6(address) => Host::Ipv6(*address),
    }
}

/// Names kept for local use: `localhost`, `local` (multicast DNS),
/// `internal` (private networks), and `home.arpa` (home networks).
const LOCAL_SUFFIXES: &[&str] = &["localhost", "local", "internal", "home.arpa"];

/// Whether `name` only resolves on this machine or its network: one with no
/// dot (`router`, found through the network's own search domains), or one
/// under a name kept for local use ([`LOCAL_SUFFIXES`]).
fn local_name(name: &str) -> bool {
    !name.contains('.')
        || LOCAL_SUFFIXES.iter().any(|suffix| {
            name.strip_suffix(suffix)
                .is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
        })
}

/// Whether an IPv4 address is outside the public internet.
fn non_global_v4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_multicast()
        // This network (0.0.0.0/8), shared address space (100.64.0.0/10),
        // protocol assignments (192.0.0.0/24), benchmarking (198.18.0.0/15),
        // and reserved (240.0.0.0/4).
        || first == 0
        || (first == 100 && (64..=127).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 198 && (18..=19).contains(&second))
        || first >= 240
}

/// Whether an IPv6 address is outside the public internet. An IPv4 address
/// carried in one is judged as itself: mapped (`::ffff:a.b.c.d`) or
/// compatible (`::a.b.c.d`, which holds `::1` and `::`), NAT64
/// (`64:ff9b::/96`), and 6to4 (`2002::/16`) addresses all reach it.
fn non_global_v6(address: Ipv6Addr) -> bool {
    if let Some(carried) = address.to_ipv4() {
        return non_global_v4(carried);
    }
    let segments = address.segments();
    let carried = |high: u16, low: u16| {
        let [a, b] = high.to_be_bytes();
        let [c, d] = low.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return non_global_v4(carried(segments[6], segments[7]));
    }
    if segments[0] == 0x2002 {
        return non_global_v4(carried(segments[1], segments[2]));
    }
    let first = segments[0];
    address.is_multicast()
        // Unique local (fc00::/7), link-local (fe80::/10), the site-local
        // range that preceded unique local (fec0::/10), and documentation
        // (2001:db8::/32).
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        || (first == 0x2001 && segments[1] == 0x0db8)
}

#[cfg(test)]
mod origins_tests;
