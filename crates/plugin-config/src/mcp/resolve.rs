//! Pure resolution of one installed MCP descriptor plus effective Setting values into the
//! Agent-independent `ResolvedMcp` the Effect source persists, plus a separate transient binding
//! set the worker holds in memory for the next Agent activation.
//!
//! The resolver owns no storage: it takes the exact installed descriptor, the revision the caller
//! expects the values to be at, and the effective values themselves, and returns one of four
//! outcomes. It depends on no SQLite, Effect, Agent runtime, or OpenCode code, so it can be
//! exercised with table-driven unit tests. P1 admits the HTTP profile only; `stdio` is a static
//! capability the runtime never materializes, so it fails closed as `UnsupportedTransport`.
//!
//! Plaintext Setting values live only in [`McpActivationBindings`]: its `Debug` redacts every
//! value, it is not `Serialize`, and the persistent [`ResolvedMcp`] carries only
//! environment-variable references and the static composition recipe. A leak through `Debug`,
//! an error, or serialization is therefore unrepresentable.

use crate::{
    CompiledMcpConfiguration, McpTransport, McpValueExpression, SettingType, SettingValue,
};
use ora_utils::hash::sha256_bytes;
use semver::Version;
use std::collections::BTreeMap;
use std::fmt;
use url::Url;

/// The exact installed MCP descriptor the resolver binds to: canonical plugin id, exact installed
/// version, and the immutable compiled configuration (which carries its own definition digest).
///
/// The backend assembles this from the installed package manifest and the compiled
/// `assets/config.json`; the resolver never reads the package itself, so it carries no file paths
/// and depends on no upper `ora-*` crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDescriptor {
    /// Canonical namespaced slug, e.g. `official/ora-space.tavily-search`.
    pub plugin_id: String,
    /// Exact installed version; bound into [`ResolvedMcp`] so a renderer can pin the definition.
    pub version: Version,
    /// Immutable compiled MCP configuration; its `definition_digest` flows into the resolved set.
    pub configuration: CompiledMcpConfiguration,
}

/// Effective Setting values bound to one `store.json` revision.
///
/// The caller reads `store.json` and passes the effective values (stored or default) together
/// with their revision, so the resolver can prove it is binding the exact revision the caller
/// expected rather than silently replaying a stale snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSettings {
    /// The revision the supplied values were actually read at.
    pub revision: u64,
    /// Effective values keyed by Setting id; defaults are already folded in by the caller.
    pub values: BTreeMap<String, SettingValue>,
}

/// One resolved HTTP header binding: the persistent recipe an Agent renderer needs.
///
/// Carries the environment-variable reference and the static prefix/suffix the adapter composes
/// with; it never carries the bound Setting value, so it is safe to persist into an Effect source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHttpHeader {
    pub name: String,
    pub env_var: String,
    pub prefix: String,
    pub suffix: String,
}

/// The persistent, plaintext-free resolved HTTP MCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHttpMcp {
    pub url: Url,
    pub headers: Vec<ResolvedHttpHeader>,
}

/// The complete resolved MCP the Effect source persists.
///
/// Agent-independent: it carries only environment references, the static composition recipe, the
/// bound definition digest, and identity — never a Setting value. The transport is always HTTP;
/// `stdio` short-circuits to [`ResolveMcp::UnsupportedTransport`] before a `ResolvedMcp` is
/// built, so a `Stdio` variant would be an unreachable state and is left unrepresented. Two
/// resolutions of the same descriptor, revision, and effective values produce equal `ResolvedMcp`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMcp {
    pub plugin_id: String,
    pub version: Version,
    pub definition_digest: String,
    pub revision: u64,
    pub transport: ResolvedHttpMcp,
}

/// The transient env-name -> raw Setting value map the worker holds in memory and hands to the
/// Agent process at activation.
///
/// Plaintext by design. Its `Debug` redacts every value, it deliberately does not implement
/// `Serialize`, and it must never be persisted, logged, or returned through an error. On a plugin
/// crash between Apply and restart the worker re-resolves from the exact revision instead of
/// trusting lost memory, so this object never needs to be durable. `PartialEq`/`Eq` compare the
/// underlying values, which never surfaces them through `Debug` or serialization.
#[derive(Clone, PartialEq, Eq)]
pub struct McpActivationBindings {
    values: BTreeMap<String, String>,
}

impl McpActivationBindings {
    /// Returns the raw value bound to one environment-variable reference, if present.
    pub fn get(&self, env_var: &str) -> Option<&str> {
        self.values.get(env_var).map(String::as_str)
    }

    /// The number of distinct environment bindings carried.
    pub fn binding_count(&self) -> usize {
        self.values.len()
    }
}

impl fmt::Debug for McpActivationBindings {
    /// Redacts every value so a stray `Debug` reach (logs, errors, panic hooks) cannot leak a key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpActivationBindings")
            .field("binding_count", &self.values.len())
            .field("values", &"<redacted: plaintext activation bindings>")
            .finish()
    }
}

/// Classifies why a required Setting cannot be used without re-prompting the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedsConfigurationReason {
    /// No effective value is available for a Setting the transport references.
    Missing,
    /// A String value is present but blank after trimming.
    Blank,
    /// The effective value's JSON type does not match the declared Setting type.
    TypeMismatch,
}

/// Reports that the MCP cannot be used until the user supplies a complete, type-correct value.
///
/// Carries only the Setting id and a stable reason code — never the offending value — so the
/// Settings UI and diagnostics stay free of plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsConfiguration {
    pub setting_id: String,
    pub reason: NeedsConfigurationReason,
}

/// The exclusive outcome of resolving one MCP descriptor against effective values.
///
/// The spec names `ResolvedMcp`, `NeedsConfiguration`, and `UnsupportedTransport`; the resolver
/// also distinguishes [`RevisionMismatch`](Self::RevisionMismatch) so a stale snapshot is re-read
/// instead of being misreported as a missing Setting. The order of checks is transport, then
/// revision, then per-Setting values, so each fault surfaces the most actionable outcome first.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolveMcp {
    /// The MCP resolved; `resolved` is safe to persist, `bindings` is transient and plaintext.
    Resolved {
        resolved: ResolvedMcp,
        bindings: McpActivationBindings,
    },
    /// A required Setting is missing, blank, or type-mismatched; the user must complete it.
    NeedsConfiguration(NeedsConfiguration),
    /// The transport is statically supported by the compiler but not by the P1 runtime (stdio).
    UnsupportedTransport,
    /// The effective values are at a different revision than the caller expected; re-snapshot.
    RevisionMismatch { expected: u64, actual: u64 },
}

impl ResolvedMcp {
    /// Computes the stable complete-set digest over a resolved MCP set.
    ///
    /// Members are sorted by plugin id then version, and headers by name, so two sets carrying the
    /// same resolved state — regardless of insertion order — digest identically. The digest input
    /// contains only environment references and static recipe text; it never contains a Setting
    /// value, so the renderer can compare digests to detect a no-op rewrite without reading a
    /// secret.
    pub fn complete_set_digest(set: &[ResolvedMcp]) -> String {
        let mut sorted: Vec<&ResolvedMcp> = set.iter().collect();
        sorted.sort_by(|left, right| {
            left.plugin_id
                .cmp(&right.plugin_id)
                .then_with(|| left.version.cmp(&right.version))
        });
        // ASCII unit separator: it cannot occur in any field the digest serializes (ids, URLs,
        // digests, and bound text reject control characters at compile time), so it is an
        // unambiguous field boundary without quoting.
        const SEP: char = '\x1f';
        let mut canonical = String::new();
        for mcp in sorted {
            canonical.push_str(&mcp.plugin_id);
            canonical.push(SEP);
            canonical.push_str(&mcp.version.to_string());
            canonical.push(SEP);
            canonical.push_str(&mcp.definition_digest);
            canonical.push(SEP);
            canonical.push_str(&mcp.revision.to_string());
            canonical.push(SEP);
            canonical.push_str(mcp.transport.url.as_str());
            canonical.push(SEP);
            let mut headers = mcp.transport.headers.iter().collect::<Vec<_>>();
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
        }
        sha256_bytes(canonical.as_bytes())
    }
}

/// Resolves one installed MCP descriptor against effective Setting values.
///
/// See the module docs for the outcome ordering and the persistent/transient split. The function
/// is total over its inputs: every failure path returns an outcome rather than panicking, and no
/// `unwrap`/`expect` is used so the strict clippy gate holds.
pub fn resolve_mcp(
    descriptor: &McpDescriptor,
    expected_revision: u64,
    effective: &EffectiveSettings,
) -> ResolveMcp {
    // Transport profile: P1 admits HTTP only. stdio is a static capability the runtime does not
    // materialize, so it fails closed as UnsupportedTransport rather than a per-Setting fault.
    let http = match &descriptor.configuration.transport {
        McpTransport::Http(http) => http,
        McpTransport::Stdio(_) => return ResolveMcp::UnsupportedTransport,
    };
    // Revision is checked before any Setting value so a stale snapshot is re-read instead of
    // surfacing NeedsConfiguration on values that are about to be replaced.
    if effective.revision != expected_revision {
        return ResolveMcp::RevisionMismatch {
            expected: expected_revision,
            actual: effective.revision,
        };
    }
    let declarations = descriptor
        .configuration
        .settings
        .as_ref()
        .map(|declaration| declaration.settings.as_slice())
        .unwrap_or(&[]);
    let mut headers = Vec::with_capacity(http.headers.len());
    let mut bindings = BTreeMap::new();
    for (position, (name, expression)) in http.headers.iter().enumerate() {
        // HTTP header values are Setting references at compile time; a Literal is structurally
        // unreachable but bound as a literal binding so the match stays exhaustive and the
        // resolver never panics on a future relaxation of the compile-time rule.
        let (setting_id, prefix, suffix, raw_value) = match expression {
            McpValueExpression::Setting { id, prefix, suffix } => {
                let Some(declaration) = declarations.iter().find(|setting| setting.id == *id)
                else {
                    return ResolveMcp::NeedsConfiguration(NeedsConfiguration {
                        setting_id: id.clone(),
                        reason: NeedsConfigurationReason::Missing,
                    });
                };
                let Some(value) = effective.values.get(id) else {
                    return ResolveMcp::NeedsConfiguration(NeedsConfiguration {
                        setting_id: id.clone(),
                        reason: NeedsConfigurationReason::Missing,
                    });
                };
                if !matches!(
                    (declaration.setting_type, value),
                    (SettingType::String, SettingValue::String(_))
                        | (SettingType::Number, SettingValue::Number(_))
                        | (SettingType::Boolean, SettingValue::Boolean(_))
                ) {
                    return ResolveMcp::NeedsConfiguration(NeedsConfiguration {
                        setting_id: id.clone(),
                        reason: NeedsConfigurationReason::TypeMismatch,
                    });
                }
                if let SettingValue::String(text) = value
                    && text.trim().is_empty()
                {
                    return ResolveMcp::NeedsConfiguration(NeedsConfiguration {
                        setting_id: id.clone(),
                        reason: NeedsConfigurationReason::Blank,
                    });
                }
                (
                    id.clone(),
                    prefix.clone(),
                    suffix.clone(),
                    setting_value_to_string(value),
                )
            }
            McpValueExpression::Literal(text) => {
                (String::new(), String::new(), String::new(), text.clone())
            }
        };
        // The binding position is unique per header, so the derived env-var name is unique within
        // this descriptor by construction; cross-MCP env-var collisions are detected at the
        // Workspace layer, which sees the full resolved set this single descriptor cannot.
        let env_var = derive_env_var(&descriptor.plugin_id, &setting_id, position);
        headers.push(ResolvedHttpHeader {
            name: name.clone(),
            env_var: env_var.clone(),
            prefix,
            suffix,
        });
        bindings.insert(env_var, raw_value);
    }
    ResolveMcp::Resolved {
        resolved: ResolvedMcp {
            plugin_id: descriptor.plugin_id.clone(),
            version: descriptor.version.clone(),
            definition_digest: descriptor.configuration.definition_digest.clone(),
            revision: effective.revision,
            transport: ResolvedHttpMcp {
                url: http.url.clone(),
                headers,
            },
        },
        bindings: McpActivationBindings { values: bindings },
    }
}

/// Converts a JSON scalar to the string every transport binding position ultimately needs.
///
/// Header values, stdio arguments, and environment values are all strings, so number and boolean
/// values are canonicalized to their `Display` form the way the compiler canonicalizes literals.
fn setting_value_to_string(value: &SettingValue) -> String {
    match value {
        SettingValue::String(text) => text.clone(),
        SettingValue::Number(number) => number.to_string(),
        SettingValue::Boolean(boolean) => boolean.to_string(),
    }
}

/// Derives a deterministic, cross-platform env-var name from the canonical plugin id, the Setting
/// id, and the binding position.
///
/// Each segment is normalized to uppercase `[A-Z0-9_]` (every non-alphanumeric byte becomes `_`)
/// so the name is valid on every target platform, and the leading `ORA_MCP_` prefix guarantees the
/// name starts with a letter. The position disambiguates two bindings of the same Setting (for
/// example the same key in two headers with different prefix/suffix), so a stable sort + digest
/// over the resolved set is reproducible.
fn derive_env_var(plugin_id: &str, setting_id: &str, position: usize) -> String {
    let mut name = String::from("ORA_MCP_");
    normalize_env_segment(plugin_id, &mut name);
    name.push('_');
    normalize_env_segment(setting_id, &mut name);
    name.push('_');
    name.push_str(&position.to_string());
    name
}

/// Upper-cases each alphanumeric byte of `input` into `out` and replaces every other byte with
/// `_`, producing a cross-platform env-var-name segment.
fn normalize_env_segment(input: &str, out: &mut String) {
    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
}
