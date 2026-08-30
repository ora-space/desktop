//! Agent-independent MCP desired State the Effect source persists, plus its plaintext-free recipe.
//!
//! Mirrors `ora_plugin_config::mcp::resolve::ResolvedMcp` in shape but is owned by the Effect
//! layer: it carries only environment-variable REFERENCES (names), the static composition recipe,
//! and identity — never a Setting value. The backend translates a resolved recipe into this type at
//! the seam so `ora-effect` never depends on `ora-plugin-config`, keeping the resource DTO
//! per-kind (Skills have [`DesiredSkillState`], MCP has [`DesiredMcpState`]) rather than sharing
//! one across both kinds. A leak through `Debug`, an error, or serialization is therefore
//! unrepresentable: every field is a reference, a digest, or a static URL or header name, and the
//! bound Setting value never enters this type.
//!
//! The persistent/transient split lives entirely in the resolver: [`DesiredMcpState`] is the
//! safe-to-persist half (env references only); the plaintext `McpActivationBindings` the worker
//! holds in memory for one Agent activation is never serialized and is re-resolved from the exact
//! revision on a plugin crash, so it never needs to be durable.

use crate::Digest;
use ora_domain::Namespace;
use serde::{Deserialize, Serialize};

/// Identifies the stable upstream MCP choice independently of its current revision.
///
/// Mirrors [`crate::SkillSelectionKey`]: the version is NOT part of the key (the key names the
/// stable plugin choice; the version identifies the head revision). Storage reuses the generic
/// `effect_sources` columns: `effect_kind = 'mcp'`, `source_kind = 'plugin'` (MCP sources are
/// always plugin-installed), and `namespace` + `identifier` carry the parsed plugin identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct McpSelectionKey {
    pub namespace: Namespace,
    pub identifier: String,
}

impl McpSelectionKey {
    /// Constructs a selection key from the parsed plugin identity halves.
    pub fn new(namespace: Namespace, identifier: impl Into<String>) -> Self {
        Self {
            namespace,
            identifier: identifier.into(),
        }
    }
}

/// One resolved HTTP header binding the Effect source persists: an env-var REFERENCE plus the
/// static prefix/suffix the adapter composes with.
///
/// It never carries the bound Setting value; `env_var` is the name the renderer resolves into a
/// process environment entry at activation time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpHttpHeaderEffect {
    pub name: String,
    pub env_var: String,
    pub prefix: String,
    pub suffix: String,
}

/// The plaintext-free resolved HTTP transport the Effect source persists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpHttpTransportEffect {
    /// Canonical HTTPS endpoint; never carries credentials (validated upstream at the resolver).
    pub url: String,
    pub headers: Vec<McpHttpHeaderEffect>,
}

/// The persistent, plaintext-free desired MCP the Effect source stores.
///
/// Agent-independent: it carries only environment references, the static composition recipe, the
/// bound definition digest, and identity — never a Setting value. The transport is always HTTP;
/// `stdio` short-circuits to `UnsupportedTransport` in the resolver before this type is built, so
/// a `Stdio` variant would be an unreachable state and is left unrepresented. Two states differing
/// only in `revision` represent two distinct resolved-value sets and digest differently.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesiredMcpState {
    pub namespace: Namespace,
    pub identifier: String,
    /// Canonical installed plugin version string (e.g. `1.0.0`); pinned so a renderer can bind it.
    pub version: String,
    /// Digest over the immutable compiled MCP definition; flows from the resolver.
    pub definition_digest: String,
    /// The `store.json` revision the effective values were bound at; pins exact-value provenance.
    pub revision: u64,
    pub transport: McpHttpTransportEffect,
}

impl DesiredMcpState {
    /// Returns the stable selection identity that indexes this desired row.
    pub fn selection_key(&self) -> McpSelectionKey {
        McpSelectionKey {
            namespace: self.namespace.clone(),
            identifier: self.identifier.clone(),
        }
    }

    /// Computes the stable per-revision content digest used as the source `state_digest`.
    ///
    /// Headers are sorted by name so two states carrying the same resolved content — regardless of
    /// insertion order — digest identically. The input contains only environment references and
    /// static recipe text (ids, digests, the URL, header names, prefixes, suffixes); it never
    /// contains a Setting value, so the renderer can compare digests to detect a no-op rewrite
    /// without ever reading a secret. A `revision` change alone digests differently, which is what
    /// lets the source store distinguish two resolved-value sets at the same definition.
    pub fn content_digest(&self) -> Digest {
        // ASCII unit separator: it cannot occur in any field the digest serializes (ids, URLs,
        // digests, and bound text reject control characters at compile time upstream), so it is an
        // unambiguous field boundary without quoting.
        const SEP: char = '\x1f';
        let mut canonical = String::new();
        canonical.push_str(self.namespace.as_ref());
        canonical.push(SEP);
        canonical.push_str(&self.identifier);
        canonical.push(SEP);
        canonical.push_str(&self.version);
        canonical.push(SEP);
        canonical.push_str(&self.definition_digest);
        canonical.push(SEP);
        canonical.push_str(&self.revision.to_string());
        canonical.push(SEP);
        canonical.push_str(&self.transport.url);
        canonical.push(SEP);
        let mut headers = self.transport.headers.iter().collect::<Vec<_>>();
        headers.sort_by(|left, right| left.name.cmp(&right.name));
        for header in headers {
            canonical.push_str(&header.name);
            canonical.push(SEP);
            canonical.push_str(&header.env_var);
            canonical.push(SEP);
            canonical.push_str(&header.prefix);
            canonical.push(SEP);
            canonical.push_str(&header.suffix);
            canonical.push(SEP);
        }
        Digest::sha256(canonical.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::{DesiredMcpState, McpHttpHeaderEffect, McpHttpTransportEffect, McpSelectionKey};
    use ora_domain::Namespace;
    use pretty_assertions::assert_eq;

    /// Builds the Tavily-shaped desired state at the given store revision.
    ///
    /// The env-var reference mirrors what the resolver derives: `ORA_MCP_` + the normalized plugin
    /// id + the normalized Setting id + the binding position. It is a NAME, never the key value.
    fn tavily(revision: u64) -> DesiredMcpState {
        DesiredMcpState {
            namespace: Namespace::new("official").unwrap(),
            identifier: "ora-space.tavily-search".to_string(),
            version: "1.0.0".to_string(),
            definition_digest: "deadbeef".to_string(),
            revision,
            transport: McpHttpTransportEffect {
                url: "https://mcp.tavily.com/mcp".to_string(),
                headers: vec![McpHttpHeaderEffect {
                    name: "Authorization".to_string(),
                    env_var: "ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0".to_string(),
                    prefix: "Bearer ".to_string(),
                    suffix: String::new(),
                }],
            },
        }
    }

    /// The selection key is the stable plugin identity, without the version or revision.
    #[test]
    fn selection_key_names_the_stable_plugin_choice_without_version() {
        assert_eq!(
            tavily(1).selection_key(),
            McpSelectionKey::new(
                Namespace::new("official").unwrap(),
                "ora-space.tavily-search",
            ),
        );
    }

    /// Two resolutions of the same state digest identically, regardless of construction order.
    #[test]
    fn content_digest_is_stable_across_reconstructions() {
        assert_eq!(tavily(1).content_digest(), tavily(1).content_digest());
    }

    /// A different store revision is a different resolved-value set and must digest differently,
    /// which is what lets the source store tell two same-definition states apart.
    #[test]
    fn content_digest_distinguishes_revisions() {
        assert_ne!(tavily(1).content_digest(), tavily(2).content_digest());
    }

    /// The digest is a `sha256:`-prefixed value over reference text only; no Setting value is in
    /// scope to leak, and the canonical form the digest covers never carries one.
    #[test]
    fn content_digest_is_a_sha256_value_over_references_only() {
        let digest = tavily(1).content_digest();
        assert_eq!(digest.as_str().len(), "sha256:".len() + 64);
        assert!(digest.as_str().starts_with("sha256:"));
    }

    /// Header sort order does not change the digest, so two equivalent resolutions agree even when
    /// the package declared its headers in a different order.
    #[test]
    fn content_digest_is_independent_of_header_order() {
        let mut state = tavily(1);
        state.transport.headers = vec![
            McpHttpHeaderEffect {
                name: "X-Trace".to_string(),
                env_var: "ORA_MCP_TRACE_0".to_string(),
                prefix: String::new(),
                suffix: String::new(),
            },
            McpHttpHeaderEffect {
                name: "Authorization".to_string(),
                env_var: "ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0".to_string(),
                prefix: "Bearer ".to_string(),
                suffix: String::new(),
            },
        ];
        let mut reordered = state.clone();
        reordered.transport.headers.reverse();
        assert_eq!(state.content_digest(), reordered.content_digest());
    }

    /// A different definition digest (the upstream definition changed) digests differently, so a
    /// stale rendered file is detectable without re-reading the package.
    #[test]
    fn content_digest_distinguishes_definitions() {
        let mut state = tavily(1);
        state.definition_digest = "cafef00d".to_string();
        assert_ne!(state.content_digest(), tavily(1).content_digest());
    }
}
