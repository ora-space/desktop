//! The decision's positive URL grammar for host MCP servers.
//!
//! The check matches the whole string character by character instead of parsing and
//! re-serializing it: URL parsers in different languages disagree on IPv4 shorthands, mapped
//! addresses, ports and dot segments, and a disagreement between Node and Controller would close
//! the control connection on every reconnect. Every accepted string is a WHATWG fixed point, so
//! the Agent's own parser sees exactly the URL the Controller sent.

/// The two schemes the grammar admits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scheme {
    Http,
    Https,
}

impl Scheme {
    /// A default port is written without the port, so spelling it out is rejected.
    const fn default_port(self) -> u32 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

/// Literal loopback hosts, the only hosts plaintext `http` may use.
const LOOPBACK_IPV4: &str = "127.0.0.1";
const LOOPBACK_IPV6: &str = "[::1]";

/// Whether `url` fully matches `scheme "://" host [":" port] path`.
pub(super) fn is_host_mcp_url(url: &str) -> bool {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (Scheme::Https, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (Scheme::Http, rest)
    } else {
        return false;
    };
    // The path is mandatory, so the authority always ends at the first `/`.
    let Some(slash) = rest.find('/') else {
        return false;
    };
    let (authority, path) = rest.split_at(slash);
    let (host, port) = match authority.strip_prefix(LOOPBACK_IPV6) {
        Some(port) => (LOOPBACK_IPV6, port),
        None => match authority.find(':') {
            Some(colon) => authority.split_at(colon),
            None => (authority, ""),
        },
    };
    let host_ok = match scheme {
        Scheme::Http => host == LOOPBACK_IPV4 || host == LOOPBACK_IPV6,
        Scheme::Https => host == LOOPBACK_IPV6 || is_ipv4(host) || is_dns_name(host),
    };
    host_ok && is_port(port, scheme) && is_path(path)
}

/// Four dotted decimal octets, each 0–255 without a leading zero.
fn is_ipv4(host: &str) -> bool {
    let mut octets = 0;
    for octet in host.split('.') {
        octets += 1;
        if !is_decimal_without_leading_zero(octet, /*max_digits*/ 3)
            || octet.parse::<u16>().map_or(true, |value| value > 255)
        {
            return false;
        }
    }
    octets == 4
}

/// Lowercase ASCII labels whose last label starts with a letter, so the name can never be read
/// as an IPv4 address in any shorthand.
fn is_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    let mut labels = host.split('.').peekable();
    while let Some(label) = labels.next() {
        let bytes = label.as_bytes();
        let label_ok = (1..=63).contains(&bytes.len())
            && bytes
                .iter()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
            && bytes.first() != Some(&b'-')
            && bytes.last() != Some(&b'-')
            && !label.starts_with("xn--");
        if !label_ok {
            return false;
        }
        if labels.peek().is_none() && !bytes[0].is_ascii_lowercase() {
            return false;
        }
    }
    true
}

/// Empty, or `:` followed by a non-default port 1–65535 without a leading zero.
fn is_port(port: &str, scheme: Scheme) -> bool {
    if port.is_empty() {
        return true;
    }
    let Some(digits) = port.strip_prefix(':') else {
        return false;
    };
    is_decimal_without_leading_zero(digits, /*max_digits*/ 5)
        && digits
            .parse::<u32>()
            .is_ok_and(|value| (1..=65_535).contains(&value) && value != scheme.default_port())
}

/// One or more `/`-prefixed segments of unreserved and sub-delimiter bytes or uppercase escapes,
/// without dot segments, which a WHATWG parser would collapse.
fn is_path(path: &str) -> bool {
    if !path.starts_with('/') || path.contains("%2E") {
        return false;
    }
    path[1..].split('/').all(|segment| {
        if segment == "." || segment == ".." {
            return false;
        }
        let bytes = segment.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'%' => {
                    let escape_ok = bytes.get(index + 1..index + 3).is_some_and(|hex| {
                        hex.iter()
                            .all(|byte| matches!(byte, b'0'..=b'9' | b'A'..=b'F'))
                    });
                    if !escape_ok {
                        return false;
                    }
                    index += 3;
                }
                byte if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&byte) => {
                    index += 1;
                }
                _ => return false,
            }
        }
        true
    })
}

/// ASCII digits, at most `max_digits`, with no leading zero except for `0` itself.
fn is_decimal_without_leading_zero(digits: &str, max_digits: usize) -> bool {
    (1..=max_digits).contains(&digits.len())
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}
