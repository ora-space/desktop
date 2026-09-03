//! AWS Signature Version 4 request-header signing for S3-compatible GET requests.
//!
//! The timestamp is injected so tests can pin a documented signing instant. SigV4 timestamps are
//! UTC because the protocol requires the `YYYYMMDD'T'HHMMSS'Z'` form; this is not an Ora clock.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use url::Url;

use super::S3Config;

type HmacSha256 = Hmac<Sha256>;

/// SHA-256 of the empty payload, used for GET requests that send no body.
pub(crate) const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SERVICE: &str = "s3";
const REQUEST_TYPE: &str = "aws4_request";
const SIGNED_HEADERS: &str = "host;x-amz-content-sha256;x-amz-date";

/// UTC instant used to build `x-amz-date` and the credential-scope date stamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SigningTime {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl SigningTime {
    /// Builds a signing instant from a UTC civil datetime.
    pub fn from_utc(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Captures the current UTC time for a live request.
    #[cfg(feature = "http-reqwest")]
    pub fn now_utc() -> Self {
        let now = time::OffsetDateTime::now_utc();
        Self {
            year: now.year(),
            month: u8::from(now.month()),
            day: now.day(),
            hour: now.hour(),
            minute: now.minute(),
            second: now.second(),
        }
    }

    /// Returns the `YYYYMMDD'T'HHMMSS'Z'` timestamp used as `x-amz-date`.
    pub fn amz_date(self) -> String {
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// Returns the `YYYYMMDD` credential-scope date stamp.
    pub fn date_stamp(self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }
}

/// Signs a path-style S3 GET of `url` using `config` at `when`.
///
/// `url` must already be the path-style object URL (`https://endpoint/bucket/key`). The returned
/// headers include `host`, `x-amz-date`, `x-amz-content-sha256`, and `authorization`.
pub fn sign_get(config: &S3Config, url: &Url, when: SigningTime) -> Vec<(String, String)> {
    let amz_date = when.amz_date();
    let date_stamp = when.date_stamp();
    let host = host_header(url);
    let canonical_request = canonical_get_request(url, &host, &amz_date);
    let scope = credential_scope(&date_stamp, config.region());
    let string_to_sign = string_to_sign(&amz_date, &scope, &canonical_request);
    let signing_key = signing_key(config.secret_key(), &date_stamp, config.region());
    let signature = hex_encode(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));
    let authorization = format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={SIGNED_HEADERS}, Signature={signature}",
        config.access_key()
    );
    vec![
        ("host".to_owned(), host),
        ("x-amz-date".to_owned(), amz_date),
        (
            "x-amz-content-sha256".to_owned(),
            EMPTY_PAYLOAD_SHA256.to_owned(),
        ),
        ("authorization".to_owned(), authorization),
    ]
}

/// Builds the SigV4 canonical request for a GET with an empty payload.
pub(crate) fn canonical_get_request(url: &Url, host: &str, amz_date: &str) -> String {
    let canonical_uri = canonical_uri(url);
    format!(
        "GET\n{canonical_uri}\n\nhost:{host}\nx-amz-content-sha256:{EMPTY_PAYLOAD_SHA256}\nx-amz-date:{amz_date}\n\n{SIGNED_HEADERS}\n{EMPTY_PAYLOAD_SHA256}"
    )
}

/// Returns the URI path used in the canonical request, defaulting to `/`.
fn canonical_uri(url: &Url) -> String {
    let path = url.path();
    if path.is_empty() {
        "/".to_owned()
    } else {
        path.to_owned()
    }
}

/// Formats the Host header, omitting the default HTTPS port.
fn host_header(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) if port != 443 => format!("{host}:{port}"),
        _ => host.to_owned(),
    }
}

/// Returns `{date}/{region}/s3/aws4_request`.
fn credential_scope(date_stamp: &str, region: &str) -> String {
    format!("{date_stamp}/{region}/{SERVICE}/{REQUEST_TYPE}")
}

/// Builds the SigV4 string-to-sign from a canonical request.
fn string_to_sign(amz_date: &str, scope: &str, canonical_request: &str) -> String {
    let hashed = hex_encode(&Sha256::digest(canonical_request.as_bytes()));
    format!("{ALGORITHM}\n{amz_date}\n{scope}\n{hashed}")
}

/// Derives the SigV4 signing key from the secret, date, and region.
fn signing_key(secret_key: &str, date_stamp: &str, region: &str) -> [u8; 32] {
    let date_key = hmac_sha256(
        format!("AWS4{secret_key}").as_bytes(),
        date_stamp.as_bytes(),
    );
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, SERVICE.as_bytes());
    hmac_sha256(&service_key, REQUEST_TYPE.as_bytes())
}

/// HMAC-SHA256; the algorithm accepts any key length so construction cannot fail.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = match HmacSha256::new_from_slice(key) {
        Ok(mac) => mac,
        Err(_) => unreachable!("HMAC-SHA256 accepts a key of any length"),
    };
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Encodes `bytes` as lowercase hexadecimal.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{
        EMPTY_PAYLOAD_SHA256, SigningTime, canonical_get_request, credential_scope, hex_encode,
        sign_get,
    };
    use crate::http::s3::S3Config;
    use pretty_assertions::assert_eq;
    use url::Url;

    fn example_config() -> S3Config {
        S3Config::new(
            "s3.example.com",
            "my-bucket",
            "us-east-1",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        )
    }

    fn example_url() -> Url {
        Url::parse("https://s3.example.com/my-bucket/plugins/pkg.orax").unwrap()
    }

    fn example_time() -> SigningTime {
        SigningTime::from_utc(2013, 5, 24, 0, 0, 0)
    }

    /// The canonical request matches the SigV4 GET layout for a path-style object URL.
    #[test]
    fn builds_the_canonical_get_request() {
        let url = example_url();
        let canonical = canonical_get_request(&url, "s3.example.com", "20130524T000000Z");
        assert_eq!(
            canonical,
            format!(
                "GET\n/my-bucket/plugins/pkg.orax\n\nhost:s3.example.com\nx-amz-content-sha256:{EMPTY_PAYLOAD_SHA256}\nx-amz-date:20130524T000000Z\n\nhost;x-amz-content-sha256;x-amz-date\n{EMPTY_PAYLOAD_SHA256}"
            )
        );
        assert_eq!(
            credential_scope("20130524", "us-east-1"),
            "20130524/us-east-1/s3/aws4_request"
        );
    }

    /// Signed headers name the host, empty-payload hash, date, and a 64-character signature.
    #[test]
    fn signs_a_path_style_get() {
        let headers = sign_get(&example_config(), &example_url(), example_time());
        let authorization = header_value(&headers, "authorization");
        assert!(authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="
        ));
        let signature = authorization.rsplit("Signature=").next().unwrap_or("");
        assert_eq!(signature.len(), 64);
        assert!(signature.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(header_value(&headers, "host"), "s3.example.com");
        assert_eq!(header_value(&headers, "x-amz-date"), "20130524T000000Z");
        assert_eq!(
            header_value(&headers, "x-amz-content-sha256"),
            EMPTY_PAYLOAD_SHA256
        );
    }

    /// Changing the secret produces a different signature so the HMAC chain is not a no-op.
    #[test]
    fn signature_depends_on_the_secret_key() {
        let original = sign_get(&example_config(), &example_url(), example_time());
        let other = S3Config::new(
            "s3.example.com",
            "my-bucket",
            "us-east-1",
            "AKIDEXAMPLE",
            "different-secret",
        );
        let changed = sign_get(&other, &example_url(), example_time());
        assert_ne!(
            header_value(&original, "authorization"),
            header_value(&changed, "authorization")
        );
    }

    /// Hex encoding is lowercase so it matches the SigV4 alphabet.
    #[test]
    fn hex_encode_is_lowercase() {
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> &'a str {
        headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    }
}
