//! The Cloud server address, reduced to the one origin a token may ever be sent to.

use thiserror::Error;
use url::Url;

/// A validated Cloud origin: `https://host[:port]`, or `http://` for a loopback host only.
///
/// The token issued by a server is bound to this exact origin. Keeping the address as an origin,
/// without a path, means there is nothing to join a hostile path onto and no second spelling of the
/// same server under which a token could be looked up or sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CloudServer {
    origin: String,
}

/// Why an entered address cannot be a Cloud server.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum InvalidServer {
    #[error("the address is not a URL")]
    NotUrl,
    #[error("the address must use https (http only for this computer)")]
    Scheme,
    #[error("the address must not carry a user name, password, path, query or fragment")]
    ExtraParts,
}

impl CloudServer {
    /// Normalizes what the user typed. A bare host gets `https://`; the host is lowercased and the
    /// default port dropped by the URL standard's origin serialization.
    pub(crate) fn parse(input: &str) -> Result<Self, InvalidServer> {
        let trimmed = input.trim();
        let candidate = if trimmed.contains("://") {
            trimmed.to_owned()
        } else {
            format!("https://{trimmed}")
        };
        let url = Url::parse(&candidate).map_err(|_| InvalidServer::NotUrl)?;
        let Some(host) = url.host() else {
            return Err(InvalidServer::NotUrl);
        };
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(InvalidServer::ExtraParts);
        }
        // Plain http is only acceptable where nothing but this computer can read the traffic.
        let loopback = match host {
            url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
            url::Host::Ipv4(address) => address.is_loopback(),
            url::Host::Ipv6(address) => address.is_loopback(),
        };
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            _ => return Err(InvalidServer::Scheme),
        }
        Ok(Self {
            origin: url.origin().ascii_serialization(),
        })
    }

    /// The origin exactly as stored and shown, e.g. `https://cloud.example`.
    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    /// An absolute URL on this origin; `path` is a fixed route of the Cloud contract.
    pub(crate) fn endpoint(&self, path: &'static str) -> String {
        format!("{}{path}", self.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::{CloudServer, InvalidServer};
    use pretty_assertions::assert_eq;

    /// Spellings of one server collapse to a single origin.
    #[test]
    fn normalizes_to_one_origin() {
        for (input, origin) in [
            ("cloud.example", "https://cloud.example"),
            (" HTTPS://Cloud.Example:443/ ", "https://cloud.example"),
            ("https://cloud.example:8443", "https://cloud.example:8443"),
            ("http://127.0.0.1:8194", "http://127.0.0.1:8194"),
            ("http://[::1]:8194/", "http://[::1]:8194"),
            ("http://localhost:5173", "http://localhost:5173"),
        ] {
            assert_eq!(
                CloudServer::parse(input).map(|server| server.origin().to_owned()),
                Ok(origin.to_owned()),
                "{input}"
            );
        }
    }

    /// Anything that could carry the token elsewhere, or over an open network, is refused.
    #[test]
    fn refuses_addresses_that_are_not_a_bare_origin() {
        for (input, error) in [
            ("", InvalidServer::NotUrl),
            ("http://cloud.example", InvalidServer::Scheme),
            ("ftp://cloud.example", InvalidServer::Scheme),
            ("https://user:pass@cloud.example", InvalidServer::ExtraParts),
            ("https://cloud.example/ora", InvalidServer::ExtraParts),
            ("https://cloud.example/?x=1", InvalidServer::ExtraParts),
            ("https://cloud.example/#top", InvalidServer::ExtraParts),
        ] {
            assert_eq!(CloudServer::parse(input), Err(error), "{input}");
        }
    }
}
