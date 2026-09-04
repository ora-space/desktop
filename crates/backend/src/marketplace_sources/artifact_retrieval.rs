use std::fmt;

use ora_contracts::{
    MarketplaceArtifactRetrieval, MarketplaceArtifactRetrievalUpdate,
    MarketplaceS3CredentialsUpdate,
};
use ora_utils::http::S3Config;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[cfg(test)]
const DIRECT_HTTPS_JSON: &str = r#"{"type":"direct_https"}"#;

const MAX_ENDPOINT_BYTES: usize = 2048;
const MAX_BUCKET_BYTES: usize = 255;
const MAX_REGION_BYTES: usize = 128;
const MAX_CREDENTIAL_BYTES: usize = 1024;

/// Holds the complete persisted artifact-retrieval state for one marketplace source.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum StoredArtifactRetrieval {
    DirectHttps,
    #[serde(rename = "s3_sigv4")]
    S3SigV4 {
        endpoint: String,
        bucket: String,
        region: String,
        access_key_id: String,
        secret_access_key: String,
    },
}

impl fmt::Debug for StoredArtifactRetrieval {
    /// Omits the entire credential pair so debug output remains safe by construction.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectHttps => formatter.write_str("DirectHttps"),
            Self::S3SigV4 {
                endpoint,
                bucket,
                region,
                ..
            } => formatter
                .debug_struct("S3SigV4")
                .field("endpoint", endpoint)
                .field("bucket", bucket)
                .field("region", region)
                .field("credentials", &"[redacted]")
                .finish(),
        }
    }
}

impl StoredArtifactRetrieval {
    /// Parses persisted JSON and revalidates every value before it enters runtime configuration.
    pub(super) fn parse(json: &str) -> Result<Self, ArtifactRetrievalError> {
        let retrieval: Self = serde_json::from_str(json)
            .map_err(|_| ArtifactRetrievalError::InvalidPersistedConfiguration)?;
        retrieval.validated()
    }

    /// Applies an editor update, preserving a credential pair only for an existing S3 source.
    pub(super) fn updated(
        existing: &Self,
        update: MarketplaceArtifactRetrievalUpdate,
    ) -> Result<Self, ArtifactRetrievalError> {
        let retrieval = match update {
            MarketplaceArtifactRetrievalUpdate::DirectHttps => Self::DirectHttps,
            MarketplaceArtifactRetrievalUpdate::S3SigV4 {
                endpoint,
                bucket,
                region,
                credentials,
            } => {
                let (access_key_id, secret_access_key) = match credentials {
                    MarketplaceS3CredentialsUpdate::Preserve => match existing {
                        Self::S3SigV4 {
                            access_key_id,
                            secret_access_key,
                            ..
                        } => (access_key_id.clone(), secret_access_key.clone()),
                        Self::DirectHttps => {
                            return Err(ArtifactRetrievalError::CredentialsRequired);
                        }
                    },
                    MarketplaceS3CredentialsUpdate::Replace {
                        access_key_id,
                        secret_access_key,
                    } => (access_key_id, secret_access_key),
                };
                Self::S3SigV4 {
                    endpoint,
                    bucket,
                    region,
                    access_key_id,
                    secret_access_key,
                }
            }
        };
        retrieval.validated()
    }

    /// Serializes the complete durable representation written to SQLite.
    pub(super) fn to_json(&self) -> Result<String, ArtifactRetrievalError> {
        serde_json::to_string(self)
            .map_err(|_| ArtifactRetrievalError::InvalidPersistedConfiguration)
    }

    /// Projects a source configuration without exposing its credential pair.
    pub(super) fn public(&self) -> MarketplaceArtifactRetrieval {
        match self {
            Self::DirectHttps => MarketplaceArtifactRetrieval::DirectHttps,
            Self::S3SigV4 {
                endpoint,
                bucket,
                region,
                ..
            } => MarketplaceArtifactRetrieval::S3SigV4 {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
            },
        }
    }

    /// Builds the generic downloader configuration when this source uses S3 SigV4.
    pub(super) fn s3_config(&self) -> Option<S3Config> {
        match self {
            Self::DirectHttps => None,
            Self::S3SigV4 {
                endpoint,
                bucket,
                region,
                access_key_id,
                secret_access_key,
            } => {
                let parsed = Url::parse(endpoint)
                    .unwrap_or_else(|_| unreachable!("stored endpoints are validated"));
                let host = parsed
                    .host_str()
                    .unwrap_or_else(|| unreachable!("stored endpoints have a host"));
                let authority = match parsed.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_owned(),
                };
                Some(S3Config::new(
                    authority,
                    bucket.clone(),
                    region.clone(),
                    access_key_id.clone(),
                    secret_access_key.clone(),
                ))
            }
        }
    }

    /// Normalizes the endpoint and rejects incomplete or ambiguous S3 states.
    fn validated(self) -> Result<Self, ArtifactRetrievalError> {
        match self {
            Self::DirectHttps => Ok(Self::DirectHttps),
            Self::S3SigV4 {
                endpoint,
                bucket,
                region,
                access_key_id,
                secret_access_key,
            } => {
                let endpoint = validated_endpoint(&endpoint)?;
                validate_scalar("bucket", &bucket, MAX_BUCKET_BYTES)?;
                if bucket.contains('/') || bucket.contains('\\') {
                    return Err(ArtifactRetrievalError::InvalidField("bucket"));
                }
                validate_scalar("region", &region, MAX_REGION_BYTES)?;
                validate_scalar("access key id", &access_key_id, MAX_CREDENTIAL_BYTES)?;
                validate_scalar(
                    "secret access key",
                    &secret_access_key,
                    MAX_CREDENTIAL_BYTES,
                )?;
                Ok(Self::S3SigV4 {
                    endpoint,
                    bucket,
                    region,
                    access_key_id,
                    secret_access_key,
                })
            }
        }
    }
}

/// Reports invalid source-scoped artifact retrieval without rendering credential values.
#[derive(Debug, Error)]
pub(crate) enum ArtifactRetrievalError {
    #[error("persisted artifact retrieval configuration is invalid")]
    InvalidPersistedConfiguration,
    #[error("S3 credentials are required when enabling S3 SigV4 retrieval")]
    CredentialsRequired,
    #[error("invalid artifact retrieval field: {0}")]
    InvalidField(&'static str),
}

/// Validates and canonicalizes an HTTPS origin used as an S3-compatible endpoint.
fn validated_endpoint(value: &str) -> Result<String, ArtifactRetrievalError> {
    if value.len() > MAX_ENDPOINT_BYTES {
        return Err(ArtifactRetrievalError::InvalidField("endpoint"));
    }
    let endpoint =
        Url::parse(value).map_err(|_| ArtifactRetrievalError::InvalidField("endpoint"))?;
    if endpoint.scheme() != "https"
        || endpoint.host_str().is_none()
        || matches!(endpoint.host(), Some(url::Host::Ipv6(_)))
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.path() != "/"
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(ArtifactRetrievalError::InvalidField("endpoint"));
    }
    Ok(endpoint.origin().ascii_serialization())
}

/// Applies shared non-empty, bounded, and printable-text constraints to one S3 scalar.
fn validate_scalar(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ArtifactRetrievalError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ArtifactRetrievalError::InvalidField(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Direct HTTPS has one compact durable and public representation.
    #[test]
    fn direct_https_round_trips() {
        let retrieval = StoredArtifactRetrieval::parse(DIRECT_HTTPS_JSON).unwrap();
        assert_eq!(retrieval, StoredArtifactRetrieval::DirectHttps);
        assert_eq!(retrieval.to_json().unwrap(), DIRECT_HTTPS_JSON);
        assert_eq!(
            retrieval.public(),
            MarketplaceArtifactRetrieval::DirectHttps
        );
    }

    /// Enabling S3 requires replacement credentials and canonicalizes the endpoint origin.
    #[test]
    fn enabling_s3_requires_complete_credentials() {
        let missing = StoredArtifactRetrieval::updated(
            &StoredArtifactRetrieval::DirectHttps,
            MarketplaceArtifactRetrievalUpdate::S3SigV4 {
                endpoint: "https://s3.example.com".to_owned(),
                bucket: "plugins".to_owned(),
                region: "region-1".to_owned(),
                credentials: MarketplaceS3CredentialsUpdate::Preserve,
            },
        );
        assert!(matches!(
            missing,
            Err(ArtifactRetrievalError::CredentialsRequired)
        ));

        let configured = StoredArtifactRetrieval::updated(
            &StoredArtifactRetrieval::DirectHttps,
            MarketplaceArtifactRetrievalUpdate::S3SigV4 {
                endpoint: "https://s3.example.com:443".to_owned(),
                bucket: "plugins".to_owned(),
                region: "region-1".to_owned(),
                credentials: MarketplaceS3CredentialsUpdate::Replace {
                    access_key_id: "access".to_owned(),
                    secret_access_key: "secret".to_owned(),
                },
            },
        )
        .unwrap();
        assert_eq!(
            configured.public(),
            MarketplaceArtifactRetrieval::S3SigV4 {
                endpoint: "https://s3.example.com".to_owned(),
                bucket: "plugins".to_owned(),
                region: "region-1".to_owned(),
            }
        );
    }

    /// Debug output never contains either member of the credential pair.
    #[test]
    fn debug_redacts_credentials() {
        let retrieval = StoredArtifactRetrieval::S3SigV4 {
            endpoint: "https://s3.example.com".to_owned(),
            bucket: "plugins".to_owned(),
            region: "region-1".to_owned(),
            access_key_id: "visible-only-to-signer".to_owned(),
            secret_access_key: "never-render-this".to_owned(),
        };
        let rendered = format!("{retrieval:?}");
        assert!(!rendered.contains("visible-only-to-signer"));
        assert!(!rendered.contains("never-render-this"));
    }
}
