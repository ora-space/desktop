//! Strict Host validation of `agent/configureWorkspace` success receipts.

#![allow(dead_code)] // First production caller is the Agent Target worker in #489.

use ora_effect::{Digest, Generation};
use ora_utils::path::{PortableRelativePath, PortableRelativePathError};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// One managed MCP entry the plugin claims to have applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpEntryReceipt {
    pub managed_identity: String,
    pub native_key: String,
    pub entry_fingerprint: Digest,
    pub source_revision_id: String,
}

/// Durably applied document the Host records after a successful configure call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpConfigurationReceipt {
    pub applied_generation: Generation,
    pub document_locator: PortableRelativePath,
    pub document_fingerprint: Digest,
    pub entries: Vec<McpEntryReceipt>,
}

/// Supported Desired entries the receipt must cover exactly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExpectedManagedMcp {
    pub managed_identity: String,
    pub source_revision_id: String,
}

/// Coverage the Host requires before treating a receipt as Ready-eligible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExpectedReceiptCoverage {
    pub generation: Generation,
    pub desired: Vec<ExpectedManagedMcp>,
}

/// Why a plugin receipt cannot advance applied or Ready generation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum ReceiptValidationError {
    #[error("configure receipt is not a JSON object")]
    NotAnObject,
    #[error("configure receipt does not match protocol v1")]
    InvalidStructure,
    #[error("configure receipt generation does not match the request")]
    GenerationMismatch,
    #[error("configure receipt locator is outside the Workspace")]
    LocatorOutOfBounds,
    #[error("configure receipt is missing a managed identity")]
    MissingManagedIdentity,
    #[error("configure receipt contains a duplicate managed identity")]
    DuplicateManagedIdentity,
    #[error("configure receipt contains a duplicate native key")]
    DuplicateNativeKey,
    #[error("configure receipt contains an extra managed identity")]
    ExtraManagedIdentity,
    #[error("configure receipt source revision does not match Desired")]
    SourceRevisionMismatch,
    #[error("configure receipt fingerprint is not a SHA-256 digest")]
    IllegalFingerprint,
}

/// Parses a plugin result before coverage checks so unknown fields cannot sneak in.
pub(crate) fn parse_mcp_configuration_receipt(
    value: Value,
) -> Result<McpConfigurationReceipt, ReceiptValidationError> {
    if !value.is_object() {
        return Err(ReceiptValidationError::NotAnObject);
    }
    let wire: WireReceipt =
        serde_json::from_value(value).map_err(|_| ReceiptValidationError::InvalidStructure)?;
    let document_locator = PortableRelativePath::parse(&wire.document_locator).map_err(
        |error: PortableRelativePathError| match error {
            PortableRelativePathError::ParentTraversal
            | PortableRelativePathError::Rooted
            | PortableRelativePathError::WindowsPrefix => {
                ReceiptValidationError::LocatorOutOfBounds
            }
            PortableRelativePathError::NulByte | PortableRelativePathError::WindowsReservedName => {
                ReceiptValidationError::InvalidStructure
            }
        },
    )?;
    if document_locator.is_root() {
        return Err(ReceiptValidationError::LocatorOutOfBounds);
    }
    let document_fingerprint = Digest::parse(wire.document_fingerprint)
        .map_err(|_| ReceiptValidationError::IllegalFingerprint)?;
    let mut entries = Vec::with_capacity(wire.entries.len());
    for entry in wire.entries {
        if entry.managed_identity.is_empty() || entry.native_key.is_empty() {
            return Err(ReceiptValidationError::InvalidStructure);
        }
        let entry_fingerprint = Digest::parse(entry.entry_fingerprint)
            .map_err(|_| ReceiptValidationError::IllegalFingerprint)?;
        entries.push(McpEntryReceipt {
            managed_identity: entry.managed_identity,
            native_key: entry.native_key,
            entry_fingerprint,
            source_revision_id: entry.source_revision_id,
        });
    }
    Ok(McpConfigurationReceipt {
        applied_generation: Generation::new(wire.applied_generation),
        document_locator,
        document_fingerprint,
        entries,
    })
}

/// Rejects receipts that would let an incomplete plugin result be marked Ready.
pub(crate) fn validate_mcp_configuration_receipt(
    receipt: &McpConfigurationReceipt,
    expected: &ExpectedReceiptCoverage,
) -> Result<(), ReceiptValidationError> {
    if receipt.applied_generation != expected.generation {
        return Err(ReceiptValidationError::GenerationMismatch);
    }

    let expected_by_id = expected
        .desired
        .iter()
        .map(|entry| {
            (
                entry.managed_identity.as_str(),
                entry.source_revision_id.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut seen_identities = BTreeSet::new();
    let mut seen_native_keys = BTreeSet::new();
    for entry in &receipt.entries {
        if !seen_identities.insert(entry.managed_identity.as_str()) {
            return Err(ReceiptValidationError::DuplicateManagedIdentity);
        }
        if !seen_native_keys.insert(entry.native_key.as_str()) {
            return Err(ReceiptValidationError::DuplicateNativeKey);
        }
        let Some(expected_revision) = expected_by_id.get(entry.managed_identity.as_str()) else {
            return Err(ReceiptValidationError::ExtraManagedIdentity);
        };
        if entry.source_revision_id != *expected_revision {
            return Err(ReceiptValidationError::SourceRevisionMismatch);
        }
    }
    if seen_identities.len() != expected.desired.len() {
        return Err(ReceiptValidationError::MissingManagedIdentity);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireReceipt {
    applied_generation: u64,
    document_locator: String,
    document_fingerprint: String,
    entries: Vec<WireEntryReceipt>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireEntryReceipt {
    managed_identity: String,
    native_key: String,
    entry_fingerprint: String,
    source_revision_id: String,
}
