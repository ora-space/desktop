use ora_plugin_protocol::{
    CandidateAuditId, ContentDigest, ContentOwnerId, JsonSafeU64, OperationId, PluginId,
    PluginVersion, parse_strict_json,
};
use serde::{Deserialize, Serialize};

pub const RECEIPT_VERSION_V1: u32 = 1;

/// Host-owned installation receipt committed atomically with staged package files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallReceipt {
    pub receipt_version: u32,
    pub plugin_id: PluginId,
    pub plugin_version: PluginVersion,
    pub source: InstallSource,
    pub installed_at_unix_ms: JsonSafeU64,
    pub content_digest: ContentDigest,
    pub file_count: JsonSafeU64,
    pub total_bytes: JsonSafeU64,
    pub operation_id: OperationId,
}

/// v1 only accepts explicitly reviewed local directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallSource {
    LocalDirectory,
}

/// The minimum persisted install fact needed to cross-check state and receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledRecord {
    pub plugin_version: PluginVersion,
    pub content_digest: ContentDigest,
    pub content_owner: ContentOwnerId,
    pub install_operation_id: OperationId,
}

/// A journaled install intent that alone authorizes reconciliation to adopt a final directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingInstall {
    pub operation_id: OperationId,
    pub plugin_id: PluginId,
    pub expected_version: PluginVersion,
    pub expected_digest: ContentDigest,
    pub candidate_audit_id: CandidateAuditId,
    pub phase: PendingInstallPhase,
}

/// Install journal phases around final-directory visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingInstallPhase {
    Prepared,
    FilesCommitted,
}

/// A journaled removal intent that keeps code disabled across crashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingRemoval {
    pub operation_id: OperationId,
    pub plugin_id: PluginId,
    pub expected_digest: ContentDigest,
    pub install_operation_id: OperationId,
    pub trash_location: String,
    pub phase: PendingRemovalPhase,
}

/// Removal journal phases around final-to-trash visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingRemovalPhase {
    Prepared,
    FilesMoved,
}

/// The closed state journal union used for bootstrap reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "operation", rename_all = "camelCase")]
pub enum PendingOperation {
    Install(PendingInstall),
    Remove(PendingRemoval),
}

/// Host-owned proof that one tombstoned final directory reached its managed trash location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemovalMarker {
    pub marker_version: u32,
    pub removal_operation_id: OperationId,
    pub plugin_id: PluginId,
    pub expected_digest: ContentDigest,
    pub install_operation_id: OperationId,
}

/// Parses a receipt through the duplicate-key/depth guard and exact v1 schema.
pub fn parse_install_receipt(bytes: &[u8]) -> Result<InstallReceipt, ReceiptError> {
    if bytes.len() > 64 * 1024 {
        return Err(ReceiptError::TooLarge);
    }
    let value = parse_strict_json(bytes, 16).map_err(|_| ReceiptError::Invalid)?;
    let receipt =
        serde_json::from_value::<InstallReceipt>(value).map_err(|_| ReceiptError::Invalid)?;
    if receipt.receipt_version != RECEIPT_VERSION_V1 {
        return Err(ReceiptError::UnsupportedVersion {
            version: receipt.receipt_version,
        });
    }
    Ok(receipt)
}

/// Parses a removal marker through the same bounded exact-JSON boundary as receipts.
pub fn parse_removal_marker(bytes: &[u8]) -> Result<RemovalMarker, ReceiptError> {
    if bytes.len() > 64 * 1024 {
        return Err(ReceiptError::TooLarge);
    }
    let value = parse_strict_json(bytes, 16).map_err(|_| ReceiptError::Invalid)?;
    let marker =
        serde_json::from_value::<RemovalMarker>(value).map_err(|_| ReceiptError::Invalid)?;
    if marker.marker_version != 1 {
        return Err(ReceiptError::UnsupportedVersion {
            version: marker.marker_version,
        });
    }
    Ok(marker)
}

/// Stable receipt parsing failures used by catalog integrity status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptError {
    #[error("receipt exceeds its byte limit")]
    TooLarge,
    #[error("receipt JSON or schema is invalid")]
    Invalid,
    #[error("receipt version {version} is unsupported")]
    UnsupportedVersion { version: u32 },
}

#[cfg(test)]
mod tests {
    use super::{InstallReceipt, InstallSource, parse_install_receipt};
    use ora_plugin_protocol::{ContentDigest, JsonSafeU64, OperationId, PluginId, PluginVersion};
    use pretty_assertions::assert_eq;

    /// Round-trips the exact v1 receipt and rejects unknown fields.
    #[test]
    fn validates_install_receipt_schema() {
        let receipt = InstallReceipt {
            receipt_version: 1,
            plugin_id: PluginId::parse("ora.example")
                .unwrap_or_else(|error| panic!("expected plugin id: {error}")),
            plugin_version: PluginVersion::parse("0.1.0")
                .unwrap_or_else(|error| panic!("expected plugin version: {error}")),
            source: InstallSource::LocalDirectory,
            installed_at_unix_ms: JsonSafeU64::new(1)
                .unwrap_or_else(|error| panic!("expected timestamp: {error}")),
            content_digest: ContentDigest::parse(format!("sha256:{}", "a".repeat(64)))
                .unwrap_or_else(|error| panic!("expected digest: {error}")),
            file_count: JsonSafeU64::new(2)
                .unwrap_or_else(|error| panic!("expected file count: {error}")),
            total_bytes: JsonSafeU64::new(3)
                .unwrap_or_else(|error| panic!("expected total bytes: {error}")),
            operation_id: OperationId::parse("550e8400-e29b-41d4-a716-446655440000")
                .unwrap_or_else(|error| panic!("expected operation id: {error}")),
        };
        let bytes = serde_json::to_vec(&receipt)
            .unwrap_or_else(|error| panic!("expected receipt serialization: {error}"));
        assert_eq!(parse_install_receipt(&bytes), Ok(receipt));

        let invalid = br#"{"receiptVersion":1,"future":true}"#;
        assert!(parse_install_receipt(invalid).is_err());
    }
}
