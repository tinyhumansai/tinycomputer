//! Plain site-name validation shared by the host and module.

/// `host` as a normalized plain site name, or `None` when it is unsafe.
///
/// The returned name is trimmed, ASCII-lowercased, and has one leading
/// `www.` removed. Its maximum byte length is 253, and it contains only ASCII
/// letters, digits, dots, and hyphens; leading dots and adjacent dots are
/// refused so the name is also a safe site-memory file stem.
#[must_use]
pub fn site_named(host: &str) -> Option<String> {
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let plain = !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('.')
        && !host.contains("..")
        && host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        // An IPv4 literal is an address, not a site: no TLD is all digits.
        && !host
            .split('.')
            .all(|label| label.bytes().all(|byte| byte.is_ascii_digit()));
    plain.then(|| host.to_owned())
}

#[cfg(test)]
#[path = "sites_tests.rs"]
mod tests;
