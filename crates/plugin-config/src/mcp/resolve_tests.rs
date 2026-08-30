//! Tests for the pure MCP resolver. The seam under test is the `resolve_mcp` function and the
//! `ResolveMcp` outcome it returns; tests never touch `store.json`, SQLite, Effect, or Agent code.

use super::{
    EffectiveSettings, McpDescriptor, NeedsConfiguration, NeedsConfigurationReason, ResolveMcp,
    ResolvedHttpHeader, ResolvedHttpMcp, ResolvedMcp, resolve_mcp,
};
use crate::{CompiledConfigurationFile, SettingValue, compile_configuration_file};
use pretty_assertions::assert_eq;
use semver::Version;
use url::Url;

/// Compiles the Tavily HTTP configuration the package ships.
fn tavily_configuration() -> crate::CompiledMcpConfiguration {
    let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "apiKey": {
                    "type": "string",
                    "title": "API key",
                    "description": "Key used to authenticate with the MCP server",
                    "required": true
                }
            },
            "transport": {
                "type": "http",
                "url": "https://mcp.tavily.com/mcp",
                "headers": {
                    "Authorization": { "setting": "apiKey", "prefix": "Bearer " }
                }
            }
        }"#;
    match compile_configuration_file(source).expect("compile Tavily configuration") {
        CompiledConfigurationFile::Mcp(compiled) => compiled,
        _ => panic!("expected the MCP shape"),
    }
}

/// Builds the Tavily descriptor the way the backend would: canonical plugin id, exact installed
/// version, and the immutable compiled configuration carrying its definition digest.
fn tavily_descriptor() -> McpDescriptor {
    McpDescriptor {
        plugin_id: "official/ora-space.tavily-search".to_string(),
        version: Version::new(1, 0, 0),
        configuration: tavily_configuration(),
    }
}

/// A complete Tavily resolve yields an Agent-independent `ResolvedMcp` carrying only an
/// environment-variable reference (never the key), plus a separate transient binding set that
/// holds the raw value. The env-var name is derived deterministically from the canonical plugin
/// id, the Setting id, and the binding position, then platform-normalized.
#[test]
fn resolves_http_mcp_with_environment_reference_and_transient_binding() {
    let descriptor = tavily_descriptor();
    let effective = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::from([(
            "apiKey".to_string(),
            SettingValue::String("tvly-test".to_string()),
        )]),
    };

    let outcome = resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective);

    let ResolveMcp::Resolved { resolved, bindings } = outcome else {
        panic!("expected a resolved MCP, got {outcome:?}");
    };

    // The persistent `ResolvedMcp` pins the exact descriptor the Agent rendered against —
    // identity, the compiler's definition digest, the bound revision, and the plaintext-free
    // transport recipe (an env-var reference plus the static prefix/suffix). Asserted as one
    // object so structural drift in any field surfaces instead of being masked by a partial check.
    assert_eq!(
        resolved,
        ResolvedMcp {
            plugin_id: "official/ora-space.tavily-search".to_string(),
            version: Version::new(1, 0, 0),
            definition_digest: descriptor.configuration.definition_digest.clone(),
            revision: 1,
            transport: ResolvedHttpMcp {
                url: Url::parse("https://mcp.tavily.com/mcp").expect("endpoint URL"),
                headers: vec![ResolvedHttpHeader {
                    name: "Authorization".to_string(),
                    env_var: "ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0".to_string(),
                    prefix: "Bearer ".to_string(),
                    suffix: String::new(),
                }],
            },
        }
    );
    // The transient binding carries the raw key value, keyed by the env-var reference.
    assert_eq!(bindings.binding_count(), 1);
    assert_eq!(
        bindings.get("ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0"),
        Some("tvly-test")
    );
    // The persistent `ResolvedMcp` carries no plaintext key: its Debug never mentions the value.
    assert!(
        !format!("{resolved:?}").contains("tvly-test"),
        "ResolvedMcp Debug must not leak the key value"
    );
    // The transient binding redacts every value in its Debug so logs and errors stay clean.
    assert!(
        !format!("{bindings:?}").contains("tvly-test"),
        "McpActivationBindings Debug must redact the key value"
    );
}

/// The complete-set digest is stable across re-resolutions of the same input and never contains
/// the key, so the renderer can detect a no-op rewrite without reading a secret.
#[test]
fn complete_set_digest_is_stable_and_free_of_plaintext() {
    let descriptor = tavily_descriptor();
    let effective = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::from([(
            "apiKey".to_string(),
            SettingValue::String("tvly-test".to_string()),
        )]),
    };
    let resolved = match resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective) {
        ResolveMcp::Resolved { resolved, .. } => resolved,
        other => panic!("expected a resolved MCP, got {other:?}"),
    };

    let digest = ResolvedMcp::complete_set_digest(std::slice::from_ref(&resolved));
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    // Re-resolving the same input yields the same digest.
    let resolved_again = match resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective) {
        ResolveMcp::Resolved { resolved, .. } => resolved,
        other => panic!("expected a resolved MCP, got {other:?}"),
    };
    assert_eq!(
        ResolvedMcp::complete_set_digest(std::slice::from_ref(&resolved_again)),
        digest
    );
    // A different resolved state (different revision) digests differently.
    let effective_two = EffectiveSettings {
        revision: 2,
        values: effective.values.clone(),
    };
    let resolved_two = match resolve_mcp(&descriptor, /*expected_revision*/ 2, &effective_two) {
        ResolveMcp::Resolved { resolved, .. } => resolved,
        other => panic!("expected a resolved MCP, got {other:?}"),
    };
    assert_ne!(
        ResolvedMcp::complete_set_digest(std::slice::from_ref(&resolved_two)),
        digest
    );
    // The digest input excludes the key: hashing the digest text must not reproduce the value.
    assert!(!digest.contains("tvly-test"));
}

/// A Setting the transport references but the user has not supplied yields `NeedsConfiguration`
/// with the `Missing` reason, so the Settings UI can point at the exact field to complete.
#[test]
fn needs_configuration_when_required_setting_is_missing() {
    let descriptor = tavily_descriptor();
    let effective = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::new(),
    };

    let outcome = resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective);

    assert_eq!(
        outcome,
        ResolveMcp::NeedsConfiguration(NeedsConfiguration {
            setting_id: "apiKey".to_string(),
            reason: NeedsConfigurationReason::Missing,
        })
    );
}

/// A present but blank String value (empty or whitespace-only) yields `Blank`, distinguishing a
/// user who opened the field but typed nothing from one who never supplied a value.
#[test]
fn needs_configuration_when_string_value_is_blank() {
    let descriptor = tavily_descriptor();
    for blank in ["", "   ", "\t\n"] {
        let effective = EffectiveSettings {
            revision: 1,
            values: std::collections::BTreeMap::from([(
                "apiKey".to_string(),
                SettingValue::String(blank.to_string()),
            )]),
        };

        assert_eq!(
            resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective),
            ResolveMcp::NeedsConfiguration(NeedsConfiguration {
                setting_id: "apiKey".to_string(),
                reason: NeedsConfigurationReason::Blank,
            }),
            "value {blank:?}"
        );
    }
}

/// A value whose JSON type does not match the declared Setting type yields `TypeMismatch`, so a
/// number stored against a String declaration is reported instead of being stringified silently.
#[test]
fn needs_configuration_when_value_type_mismatches_declaration() {
    let descriptor = tavily_descriptor();
    let effective = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::from([(
            "apiKey".to_string(),
            SettingValue::Number(123.into()),
        )]),
    };

    assert_eq!(
        resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective),
        ResolveMcp::NeedsConfiguration(NeedsConfiguration {
            setting_id: "apiKey".to_string(),
            reason: NeedsConfigurationReason::TypeMismatch,
        })
    );
}

/// A `NeedsConfiguration` outcome carries only the Setting id and a reason code; the offending
/// value (present in the type-mismatch case) must never appear in the outcome's `Debug`.
#[test]
fn needs_configuration_outcome_carries_no_plaintext_value() {
    let descriptor = tavily_descriptor();
    let effective = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::from([(
            "apiKey".to_string(),
            SettingValue::String("tvly-secret".to_string()),
        )]),
    };
    // A present, type-correct, non-blank value resolves; to exercise a value-bearing failure we
    // force a type mismatch by passing a number against the String declaration.
    let mismatched = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::from([(
            "apiKey".to_string(),
            SettingValue::Number(999.into()),
        )]),
    };

    let outcome = resolve_mcp(&descriptor, /*expected_revision*/ 1, &mismatched);
    let debug = format!("{outcome:?}");
    assert!(
        !debug.contains("999"),
        "NeedsConfiguration Debug must not leak the value: {debug}"
    );
    // The complete outcome enum (including the resolved variant) must not surface the key either.
    let resolved_outcome = resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective);
    assert!(
        !format!("{resolved_outcome:?}").contains("tvly-secret"),
        "ResolveMcp Debug must not leak the key value"
    );
}

/// When the effective values are at a different revision than the caller expected, the resolver
/// returns `RevisionMismatch` in both directions so the caller re-snapshots instead of binding a
/// stale revision or misreporting a missing Setting on values that are about to be replaced.
#[test]
fn revision_mismatch_when_effective_revision_differs_from_expected() {
    let descriptor = tavily_descriptor();
    let values = std::collections::BTreeMap::from([(
        "apiKey".to_string(),
        SettingValue::String("tvly-test".to_string()),
    )]);

    // Values moved ahead of the expected revision.
    let ahead = EffectiveSettings {
        revision: 2,
        values: values.clone(),
    };
    assert_eq!(
        resolve_mcp(&descriptor, /*expected_revision*/ 1, &ahead),
        ResolveMcp::RevisionMismatch {
            expected: 1,
            actual: 2,
        }
    );
    // Values lagging behind the expected revision.
    let behind = EffectiveSettings {
        revision: 1,
        values,
    };
    assert_eq!(
        resolve_mcp(&descriptor, /*expected_revision*/ 2, &behind),
        ResolveMcp::RevisionMismatch {
            expected: 2,
            actual: 1,
        }
    );
}

/// A stdio transport is statically supported by the compiler but not materialized by the P1
/// runtime, so it fails closed as `UnsupportedTransport` — and that check takes priority over a
/// stale revision or missing Setting, since the transport is a property of the descriptor.
#[test]
fn stdio_transport_is_unsupported_and_takes_priority_over_revision_or_settings() {
    let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "repository": {"type":"string","title":"Repository","description":"Repo","required":true}
            },
            "transport": {
                "type": "stdio",
                "command": "assets/server",
                "args": ["--repo", { "setting": "repository" }]
            }
        }"#;
    let configuration = match compile_configuration_file(source).expect("compile stdio") {
        CompiledConfigurationFile::Mcp(compiled) => compiled,
        _ => panic!("expected the MCP shape"),
    };
    let descriptor = McpDescriptor {
        plugin_id: "official/example-stdio".to_string(),
        version: Version::new(0, 1, 0),
        configuration,
    };

    // Even with a matching revision and a complete value, stdio is unsupported.
    let complete = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::from([(
            "repository".to_string(),
            SettingValue::String("owner/name".to_string()),
        )]),
    };
    assert_eq!(
        resolve_mcp(&descriptor, /*expected_revision*/ 1, &complete),
        ResolveMcp::UnsupportedTransport
    );
    // The transport check fires before the revision check (stale revision would still be stdio).
    let stale = EffectiveSettings {
        revision: 2,
        values: complete.values.clone(),
    };
    assert_eq!(
        resolve_mcp(&descriptor, /*expected_revision*/ 1, &stale),
        ResolveMcp::UnsupportedTransport
    );
    // And before the per-Setting value check (missing value would still be stdio).
    let missing = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::new(),
    };
    assert_eq!(
        resolve_mcp(&descriptor, /*expected_revision*/ 1, &missing),
        ResolveMcp::UnsupportedTransport
    );
}

/// Prefix and suffix are static package text the resolved header carries verbatim, while the
/// transient binding holds the raw Setting value — the adapter composes `prefix + value + suffix`
/// at render time, so the persistent state and the binding set stay free of one another's data.
#[test]
fn prefix_and_suffix_are_carried_and_binding_holds_raw_value() {
    let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "token": {"type":"string","title":"Token","description":"Cred","required":true}
            },
            "transport": {
                "type": "http",
                "url": "https://mcp.example.com/v1",
                "headers": {
                    "X-Token": { "setting": "token", "prefix": "schemed ", "suffix": " trailing" }
                }
            }
        }"#;
    let configuration = match compile_configuration_file(source).expect("compile") {
        CompiledConfigurationFile::Mcp(compiled) => compiled,
        _ => panic!("expected the MCP shape"),
    };
    let descriptor = McpDescriptor {
        plugin_id: "official/prefixed-http".to_string(),
        version: Version::new(1, 2, 0),
        configuration,
    };
    let effective = EffectiveSettings {
        revision: 3,
        values: std::collections::BTreeMap::from([(
            "token".to_string(),
            SettingValue::String("raw-value".to_string()),
        )]),
    };

    let ResolveMcp::Resolved { resolved, bindings } =
        resolve_mcp(&descriptor, /*expected_revision*/ 3, &effective)
    else {
        panic!("expected a resolved MCP");
    };

    // The resolved header carries the static prefix/suffix verbatim; the transient binding holds
    // the raw Setting value, never the composed `schemed raw-value trailing`.
    assert_eq!(
        resolved.transport.headers[0],
        ResolvedHttpHeader {
            name: "X-Token".to_string(),
            env_var: "ORA_MCP_OFFICIAL_PREFIXED_HTTP_TOKEN_0".to_string(),
            prefix: "schemed ".to_string(),
            suffix: " trailing".to_string(),
        }
    );
    assert_eq!(
        bindings.get("ORA_MCP_OFFICIAL_PREFIXED_HTTP_TOKEN_0"),
        Some("raw-value")
    );
}

/// The complete-set digest is order-independent: the same resolved set digests identically
/// regardless of insertion order, because members are sorted by plugin id then version.
#[test]
fn complete_set_digest_is_order_independent_across_members() {
    let tavily = match resolve_mcp(
        &tavily_descriptor(),
        /*expected_revision*/ 1,
        &EffectiveSettings {
            revision: 1,
            values: std::collections::BTreeMap::from([(
                "apiKey".to_string(),
                SettingValue::String("tvly-test".to_string()),
            )]),
        },
    ) {
        ResolveMcp::Resolved { resolved, .. } => resolved,
        other => panic!("expected a resolved MCP, got {other:?}"),
    };
    let other_source = br#"{
            "schemaVersion": 1,
            "transport": { "type": "http", "url": "https://mcp.example.com/v1" }
        }"#;
    let other_configuration = match compile_configuration_file(other_source).expect("compile") {
        CompiledConfigurationFile::Mcp(compiled) => compiled,
        _ => panic!("expected the MCP shape"),
    };
    let other = match resolve_mcp(
        &McpDescriptor {
            plugin_id: "official/aaa-transport-only".to_string(),
            version: Version::new(0, 1, 0),
            configuration: other_configuration,
        },
        /*expected_revision*/ 1,
        &EffectiveSettings {
            revision: 1,
            values: std::collections::BTreeMap::new(),
        },
    ) {
        ResolveMcp::Resolved { resolved, .. } => resolved,
        other => panic!("expected a resolved MCP, got {other:?}"),
    };

    let direct = ResolvedMcp::complete_set_digest(&[tavily.clone(), other.clone()]);
    let reversed = ResolvedMcp::complete_set_digest(&[other, tavily]);
    assert_eq!(direct, reversed);
    assert!(direct.starts_with(|c: char| c.is_ascii_hexdigit()));
}

/// Two headers bound to the same Setting get distinct env-var names (disambiguated by binding
/// position) and distinct transient bindings carrying the same raw value, ordered by header name.
#[test]
fn multiple_headers_share_a_setting_with_distinct_positions() {
    let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "apiKey": {"type":"string","title":"API key","description":"Key","required":true}
            },
            "transport": {
                "type": "http",
                "url": "https://mcp.example.com/v1",
                "headers": {
                    "X-Api-Key": { "setting": "apiKey" },
                    "Authorization": { "setting": "apiKey", "prefix": "Bearer " }
                }
            }
        }"#;
    let configuration = match compile_configuration_file(source).expect("compile") {
        CompiledConfigurationFile::Mcp(compiled) => compiled,
        _ => panic!("expected the MCP shape"),
    };
    let descriptor = McpDescriptor {
        plugin_id: "official/dual-header".to_string(),
        version: Version::new(1, 0, 0),
        configuration,
    };
    let effective = EffectiveSettings {
        revision: 1,
        values: std::collections::BTreeMap::from([(
            "apiKey".to_string(),
            SettingValue::String("tvly-test".to_string()),
        )]),
    };

    let ResolveMcp::Resolved { resolved, bindings } =
        resolve_mcp(&descriptor, /*expected_revision*/ 1, &effective)
    else {
        panic!("expected a resolved MCP");
    };

    // Headers are emitted in BTreeMap (sorted-by-name) order: Authorization before X-Api-Key.
    assert_eq!(resolved.transport.headers.len(), 2);
    let [first, second] = &resolved.transport.headers[..] else {
        panic!("expected exactly two headers");
    };
    assert_eq!(first.name, "Authorization");
    assert_eq!(second.name, "X-Api-Key");
    // Same Setting, different positions -> distinct env-var names.
    assert_ne!(first.env_var, second.env_var);
    assert!(first.env_var.ends_with("_APIKEY_0"));
    assert!(second.env_var.ends_with("_APIKEY_1"));
    // Both bindings carry the same raw value; neither carries the composed `Bearer ` form.
    assert_eq!(bindings.get(&first.env_var), Some("tvly-test"));
    assert_eq!(bindings.get(&second.env_var), Some("tvly-test"));
}
