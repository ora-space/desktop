use crate::{InvalidFieldReason, ManifestError, ManifestField};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The `{home}` placeholder: the user's home directory.
const PLACEHOLDER_HOME: &str = "{home}";
/// The `{data_dir}` placeholder: the per-OS user data directory (`XDG_DATA_HOME` on Linux).
const PLACEHOLDER_DATA_DIR: &str = "{data_dir}";
/// The `{agent_session_id}` placeholder: the agent-side session id the host substitutes.
const PLACEHOLDER_AGENT_SESSION_ID: &str = "{agent_session_id}";

const MAX_FORMAT_BYTES: usize = 64;
const MAX_TEMPLATE_BYTES: usize = 1024;
const MAX_SESSION_ID_BYTES: usize = 128;

/// Holds the validated `[agent]` section of an agent-kind plugin manifest.
///
/// The section is optional: an agent whose runtime produces no trace simply omits it. It is
/// rejected on every other plugin kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginAgent {
    pub(crate) trace: Option<PluginAgentTrace>,
}

impl PluginAgent {
    /// Returns the declared trace locator, when this agent produces trace files.
    pub fn trace(&self) -> Option<&PluginAgentTrace> {
        self.trace.as_ref()
    }
}

/// Declares where one agent's trace files live and how to find the file of a given session.
///
/// `format` is a passthrough identifier the dashboard selects its parser with; the host never
/// interprets it. `locator` resolves to one concrete file (`file` template) or to a rooted glob
/// the host scans (`search` form), and validation guarantees the resolved result can only reach
/// paths under `{home}` or `{data_dir}`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginAgentTrace {
    pub(crate) format: AgentTraceFormat,
    pub(crate) locator: AgentTraceLocatorTemplate,
}

impl PluginAgentTrace {
    /// Returns the passthrough format identifier (for example `claude_code` or `opencode`).
    pub fn format(&self) -> &str {
        self.format.as_str()
    }

    /// Substitutes the declared template with one session's context.
    ///
    /// The session id is validated before substitution: it may contain only `[A-Za-z0-9_-]`, so
    /// a hostile id can never smuggle path separators or `..` through a template.
    pub fn resolve(
        &self,
        context: &TraceResolveContext<'_>,
    ) -> Result<TraceLocator, TraceResolutionError> {
        validate_session_id(context.agent_session_id)?;
        match &self.locator {
            AgentTraceLocatorTemplate::File { template } => Ok(TraceLocator::File {
                path: PathBuf::from(substitute(
                    template.as_str(),
                    context,
                    &[
                        PLACEHOLDER_HOME,
                        PLACEHOLDER_DATA_DIR,
                        PLACEHOLDER_AGENT_SESSION_ID,
                    ],
                )?),
            }),
            AgentTraceLocatorTemplate::Search { root, pattern } => {
                let root = PathBuf::from(substitute(
                    root.as_str(),
                    context,
                    &[PLACEHOLDER_HOME, PLACEHOLDER_DATA_DIR],
                )?);
                let pattern =
                    substitute(pattern.as_str(), context, &[PLACEHOLDER_AGENT_SESSION_ID])?;
                Ok(TraceLocator::Search { root, pattern })
            }
        }
    }
}

/// The substituted result of one trace declaration: either one exact file or a glob to scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceLocator {
    /// One exact file path, fully substituted with the session context.
    File { path: PathBuf },
    /// A rooted glob search: `pattern` is relative to `root` and contains no placeholders.
    Search { root: PathBuf, pattern: String },
}

/// Carries the values the host substitutes into a trace template.
pub struct TraceResolveContext<'a> {
    /// The user's home directory, absolute.
    pub home: &'a Path,
    /// The per-OS user data directory, absolute.
    pub data_dir: &'a Path,
    /// The agent-side session id of the session whose trace is being located.
    pub agent_session_id: &'a str,
}

/// Reports why a trace declaration could not be resolved for one session.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TraceResolutionError {
    #[error("agent session id contains unsafe characters: {found:?}")]
    UnsafeSessionId { found: String },
    #[error("agent session id must not be empty")]
    EmptySessionId,
    #[error("agent session id exceeds {max_bytes} bytes: {actual_bytes}")]
    SessionIdTooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
}

/// A trace format identifier: lowercase `[a-z][a-z0-9_]*`, for example `claude_code`.
///
/// The host passes it through without interpreting it; the dashboard selects its parser with it.
/// It is still validated (not a free string) so a typo fails at install rather than at render.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AgentTraceFormat(String);

impl AgentTraceFormat {
    /// Parses one format identifier without normalizing it.
    pub fn parse(value: &str) -> Result<Self, AgentTraceFormatError> {
        if value.is_empty() {
            return Err(AgentTraceFormatError::Empty);
        }
        if value.len() > MAX_FORMAT_BYTES {
            return Err(AgentTraceFormatError::TooLong {
                max_bytes: MAX_FORMAT_BYTES,
                actual_bytes: value.len(),
            });
        }
        let mut characters = value.chars();
        if !characters
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        {
            return Err(AgentTraceFormatError::InvalidLeadingCharacter);
        }
        if let Some(character) =
            characters.find(|character| !matches!(character, 'a'..='z' | '0'..='9' | '_'))
        {
            return Err(AgentTraceFormatError::InvalidCharacter { character });
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the format identifier as spelled in the manifest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reports why a trace format identifier was rejected.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AgentTraceFormatError {
    #[error("trace format must not be empty")]
    Empty,
    #[error("trace format exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("trace format must start with a lowercase letter")]
    InvalidLeadingCharacter,
    #[error("trace format contains invalid character {character:?}")]
    InvalidCharacter { character: char },
}

/// The two mutually exclusive locator forms of a `[agent.trace]` section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentTraceLocatorTemplate {
    /// One path template containing `{agent_session_id}` in its final component.
    File { template: TraceFileTemplate },
    /// A fixed root plus a session-relative glob.
    Search {
        root: TraceRootTemplate,
        pattern: TraceSearchPattern,
    },
}

/// A validated single-file trace path template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceFileTemplate(String);

impl TraceFileTemplate {
    /// Returns the template as spelled in the manifest, placeholders included.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated search root template: anchored at `{home}` or `{data_dir}`, no session id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRootTemplate(String);

impl TraceRootTemplate {
    /// Returns the template as spelled in the manifest, placeholders included.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated session-relative glob pattern.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceSearchPattern(String);

impl TraceSearchPattern {
    /// Returns the pattern as spelled in the manifest, placeholders included.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mirrors `[agent]` before semantic validation; unknown fields fail structurally.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAgent {
    trace: Option<RawAgentTrace>,
}

/// Mirrors `[agent.trace]` before semantic validation; unknown fields fail structurally.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAgentTrace {
    format: String,
    file: Option<String>,
    search: Option<RawAgentTraceSearch>,
}

/// Mirrors `[agent.trace.search]` before semantic validation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAgentTraceSearch {
    root: String,
    glob: String,
}

impl TryFrom<RawAgent> for PluginAgent {
    type Error = ManifestError;

    /// Validates the optional trace declaration in field order.
    fn try_from(raw: RawAgent) -> Result<Self, Self::Error> {
        let trace = raw.trace.map(PluginAgentTrace::try_from).transpose()?;
        Ok(Self { trace })
    }
}

impl TryFrom<RawAgentTrace> for PluginAgentTrace {
    type Error = ManifestError;

    /// Validates the format and exactly one of the two locator forms.
    fn try_from(raw: RawAgentTrace) -> Result<Self, Self::Error> {
        let format =
            AgentTraceFormat::parse(&raw.format).map_err(|reason| ManifestError::InvalidField {
                field: ManifestField::AgentTraceFormat,
                reason: reason.into(),
            })?;

        let locator = match (raw.file, raw.search) {
            (Some(template), None) => {
                validate_file_template(&template)?;
                AgentTraceLocatorTemplate::File {
                    template: TraceFileTemplate(template),
                }
            }
            (None, Some(search)) => AgentTraceLocatorTemplate::Search {
                root: validate_root_template(&search.root)?,
                pattern: validate_search_pattern(&search.glob)?,
            },
            (Some(_), Some(_)) => {
                return Err(ManifestError::InvalidField {
                    field: ManifestField::AgentTraceLocator,
                    reason: InvalidFieldReason::FileAndSearchConflict,
                });
            }
            (None, None) => {
                return Err(ManifestError::InvalidField {
                    field: ManifestField::AgentTraceLocator,
                    reason: InvalidFieldReason::MissingFileOrSearch,
                });
            }
        };

        Ok(Self { format, locator })
    }
}

/// Validates a single-file template: session-id placeholder required, safe segments only.
fn validate_file_template(template: &str) -> Result<(), ManifestError> {
    validate_template_length(template, ManifestField::AgentTraceFile)?;
    validate_no_parent_segments(template, ManifestField::AgentTraceFile)?;
    validate_placeholders(
        template,
        &[
            PLACEHOLDER_HOME,
            PLACEHOLDER_DATA_DIR,
            PLACEHOLDER_AGENT_SESSION_ID,
        ],
        true,
        ManifestField::AgentTraceFile,
    )
}

/// Validates a search root: anchored at `{home}`/`{data_dir}`, no session-id placeholder.
fn validate_root_template(root: &str) -> Result<TraceRootTemplate, ManifestError> {
    validate_template_length(root, ManifestField::AgentTraceSearchRoot)?;
    validate_no_parent_segments(root, ManifestField::AgentTraceSearchRoot)?;
    let anchored = root.starts_with(PLACEHOLDER_HOME) || root.starts_with(PLACEHOLDER_DATA_DIR);
    if !anchored {
        return Err(ManifestError::InvalidField {
            field: ManifestField::AgentTraceSearchRoot,
            reason: InvalidFieldReason::RootMustBeAnchored,
        });
    }
    validate_placeholders(
        root,
        &[PLACEHOLDER_HOME, PLACEHOLDER_DATA_DIR],
        false,
        ManifestField::AgentTraceSearchRoot,
    )?;
    Ok(TraceRootTemplate(root.to_owned()))
}

/// Validates a search pattern: relative, session-id placeholder required, safe segments only.
fn validate_search_pattern(pattern: &str) -> Result<TraceSearchPattern, ManifestError> {
    validate_template_length(pattern, ManifestField::AgentTraceSearchPattern)?;
    if pattern.starts_with('/') || pattern.starts_with('\\') {
        return Err(ManifestError::InvalidField {
            field: ManifestField::AgentTraceSearchPattern,
            reason: InvalidFieldReason::PatternMustBeRelative,
        });
    }
    validate_no_parent_segments(pattern, ManifestField::AgentTraceSearchPattern)?;
    validate_placeholders(
        pattern,
        &[PLACEHOLDER_AGENT_SESSION_ID],
        true,
        ManifestField::AgentTraceSearchPattern,
    )?;
    Ok(TraceSearchPattern(pattern.to_owned()))
}

/// Applies the shared byte-length bound to every trace template.
fn validate_template_length(value: &str, field: ManifestField) -> Result<(), ManifestError> {
    if value.is_empty() {
        return Err(ManifestError::InvalidField {
            field,
            reason: InvalidFieldReason::Empty,
        });
    }
    if value.len() > MAX_TEMPLATE_BYTES {
        return Err(ManifestError::InvalidField {
            field,
            reason: InvalidFieldReason::TooLong {
                max_bytes: MAX_TEMPLATE_BYTES,
                actual_bytes: value.len(),
            },
        });
    }
    Ok(())
}

/// Rejects templates containing a `..` segment: every substituted value is safe, but a literal
/// `..` could escape the intended base directory.
fn validate_no_parent_segments(value: &str, field: ManifestField) -> Result<(), ManifestError> {
    let is_separator = |character: char| character == '/' || character == '\\';
    for segment in value.split(is_separator) {
        if segment == ".." {
            return Err(ManifestError::InvalidField {
                field,
                reason: InvalidFieldReason::ContainsParentSegment,
            });
        }
    }
    Ok(())
}

/// Checks every `{...}` occurrence against the allowed set, and enforces the session-id
/// placeholder when `require_session_id` is set.
fn validate_placeholders(
    value: &str,
    allowed: &[&str],
    require_session_id: bool,
    field: ManifestField,
) -> Result<(), ManifestError> {
    let mut found_session_id = false;
    let mut rest = value;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open + 1..].find('}').map(|offset| open + 1 + offset) else {
            return Err(ManifestError::InvalidField {
                field,
                reason: InvalidFieldReason::UnknownPlaceholder {
                    found: rest[open..].to_owned(),
                },
            });
        };
        let placeholder = &rest[open..=close];
        if !allowed.contains(&placeholder) {
            return Err(ManifestError::InvalidField {
                field,
                reason: InvalidFieldReason::UnknownPlaceholder {
                    found: placeholder.to_owned(),
                },
            });
        }
        if placeholder == PLACEHOLDER_AGENT_SESSION_ID {
            found_session_id = true;
        }
        rest = &rest[close + 1..];
    }
    if require_session_id && !found_session_id {
        return Err(ManifestError::InvalidField {
            field,
            reason: InvalidFieldReason::MissingRequiredPlaceholder {
                placeholder: PLACEHOLDER_AGENT_SESSION_ID,
            },
        });
    }
    Ok(())
}

/// Substitutes every occurrence of the allowed placeholders with the session context.
///
/// Callers pass the exact set of placeholders the template was validated with, so no unknown
/// placeholder can survive to resolution.
fn substitute(
    template: &str,
    context: &TraceResolveContext<'_>,
    allowed: &[&str],
) -> Result<String, TraceResolutionError> {
    let mut output = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        output.push_str(&rest[..open]);
        let close = rest[open + 1..].find('}').expect("template was validated") + open + 1;
        let placeholder = &rest[open..=close];
        let replacement = match placeholder {
            PLACEHOLDER_HOME => context.home.to_string_lossy().into_owned(),
            PLACEHOLDER_DATA_DIR => context.data_dir.to_string_lossy().into_owned(),
            PLACEHOLDER_AGENT_SESSION_ID => context.agent_session_id.to_owned(),
            found => {
                return Err(TraceResolutionError::UnsafeSessionId {
                    found: found.to_owned(),
                });
            }
        };
        let _ = allowed; // The validated set decides membership; the match above is exhaustive.
        output.push_str(&replacement);
        rest = &rest[close + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

/// Rejects session ids that could turn a template into a path escape.
fn validate_session_id(session_id: &str) -> Result<(), TraceResolutionError> {
    if session_id.is_empty() {
        return Err(TraceResolutionError::EmptySessionId);
    }
    if session_id.len() > MAX_SESSION_ID_BYTES {
        return Err(TraceResolutionError::SessionIdTooLong {
            max_bytes: MAX_SESSION_ID_BYTES,
            actual_bytes: session_id.len(),
        });
    }
    if let Some(_character) = session_id
        .chars()
        .find(|character| !matches!(character, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_'))
    {
        return Err(TraceResolutionError::UnsafeSessionId {
            found: session_id.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_formats_and_rejects_invalid_ones() {
        for valid in ["claude_code", "opencode", "gemini", "a1_b"] {
            assert!(
                AgentTraceFormat::parse(valid).is_ok(),
                "expected {valid} to parse"
            );
        }
        for invalid in ["", "Claude_Code", "1abc", "a-b", "a.b", "a$b"] {
            assert!(
                AgentTraceFormat::parse(invalid).is_err(),
                "expected {invalid} to fail"
            );
        }
    }

    #[test]
    fn file_template_resolves_with_session_context() {
        let trace = PluginAgentTrace::try_from(RawAgentTrace {
            format: "opencode".to_owned(),
            file: Some("{data_dir}/opencode/trace/{agent_session_id}.ndjson".to_owned()),
            search: None,
        })
        .unwrap();
        let context = TraceResolveContext {
            home: Path::new("/home/user"),
            data_dir: Path::new("/home/user/.local/share"),
            agent_session_id: "ses_abc123",
        };
        let locator = trace.resolve(&context).unwrap();
        assert_eq!(
            locator,
            TraceLocator::File {
                path: PathBuf::from("/home/user/.local/share/opencode/trace/ses_abc123.ndjson"),
            }
        );
    }

    #[test]
    fn search_template_resolves_to_rooted_pattern() {
        let trace = PluginAgentTrace::try_from(RawAgentTrace {
            format: "claude_code".to_owned(),
            file: None,
            search: Some(RawAgentTraceSearch {
                root: "{home}/.claude/projects".to_owned(),
                glob: "**/{agent_session_id}.jsonl".to_owned(),
            }),
        })
        .unwrap();
        let context = TraceResolveContext {
            home: Path::new("/home/user"),
            data_dir: Path::new("/home/user/.local/share"),
            agent_session_id: "1f1f2a2e-0000-4b7c-9f3d-1234567890ab",
        };
        let locator = trace.resolve(&context).unwrap();
        assert_eq!(
            locator,
            TraceLocator::Search {
                root: PathBuf::from("/home/user/.claude/projects"),
                pattern: "**/1f1f2a2e-0000-4b7c-9f3d-1234567890ab.jsonl".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_file_template_without_session_id_placeholder() {
        let error = PluginAgentTrace::try_from(RawAgentTrace {
            format: "opencode".to_owned(),
            file: Some("{data_dir}/opencode/trace/all.ndjson".to_owned()),
            search: None,
        })
        .unwrap_err();
        assert!(matches!(
            error,
            ManifestError::InvalidField {
                field: ManifestField::AgentTraceFile,
                reason: InvalidFieldReason::MissingRequiredPlaceholder {
                    placeholder: "{agent_session_id}"
                },
            }
        ));
    }

    #[test]
    fn rejects_unknown_placeholders_and_parent_segments() {
        let unknown = PluginAgentTrace::try_from(RawAgentTrace {
            format: "opencode".to_owned(),
            file: Some("{home}/{agent_session_id}/{workspace}.ndjson".to_owned()),
            search: None,
        })
        .unwrap_err();
        assert!(matches!(
            unknown,
            ManifestError::InvalidField {
                field: ManifestField::AgentTraceFile,
                reason: InvalidFieldReason::UnknownPlaceholder { .. },
            }
        ));

        let parent = PluginAgentTrace::try_from(RawAgentTrace {
            format: "opencode".to_owned(),
            file: Some("{home}/../../{agent_session_id}.ndjson".to_owned()),
            search: None,
        })
        .unwrap_err();
        assert!(matches!(
            parent,
            ManifestError::InvalidField {
                field: ManifestField::AgentTraceFile,
                reason: InvalidFieldReason::ContainsParentSegment,
            }
        ));
    }

    #[test]
    fn rejects_unanchored_search_root_and_absolute_pattern() {
        let unanchored = PluginAgentTrace::try_from(RawAgentTrace {
            format: "claude_code".to_owned(),
            file: None,
            search: Some(RawAgentTraceSearch {
                root: "/tmp/traces".to_owned(),
                glob: "**/{agent_session_id}.jsonl".to_owned(),
            }),
        })
        .unwrap_err();
        assert!(matches!(
            unanchored,
            ManifestError::InvalidField {
                field: ManifestField::AgentTraceSearchRoot,
                reason: InvalidFieldReason::RootMustBeAnchored,
            }
        ));

        let absolute = PluginAgentTrace::try_from(RawAgentTrace {
            format: "claude_code".to_owned(),
            file: None,
            search: Some(RawAgentTraceSearch {
                root: "{home}/.claude/projects".to_owned(),
                glob: "/etc/{agent_session_id}.jsonl".to_owned(),
            }),
        })
        .unwrap_err();
        assert!(matches!(
            absolute,
            ManifestError::InvalidField {
                field: ManifestField::AgentTraceSearchPattern,
                reason: InvalidFieldReason::PatternMustBeRelative,
            }
        ));
    }

    #[test]
    fn requires_exactly_one_locator_form() {
        let both = PluginAgentTrace::try_from(RawAgentTrace {
            format: "opencode".to_owned(),
            file: Some("{data_dir}/a/{agent_session_id}.ndjson".to_owned()),
            search: Some(RawAgentTraceSearch {
                root: "{home}/a".to_owned(),
                glob: "**/{agent_session_id}.jsonl".to_owned(),
            }),
        })
        .unwrap_err();
        assert!(matches!(
            both,
            ManifestError::InvalidField {
                field: ManifestField::AgentTraceLocator,
                reason: InvalidFieldReason::FileAndSearchConflict,
            }
        ));

        let neither = PluginAgentTrace::try_from(RawAgentTrace {
            format: "opencode".to_owned(),
            file: None,
            search: None,
        })
        .unwrap_err();
        assert!(matches!(
            neither,
            ManifestError::InvalidField {
                field: ManifestField::AgentTraceLocator,
                reason: InvalidFieldReason::MissingFileOrSearch,
            }
        ));
    }

    #[test]
    fn resolution_rejects_unsafe_session_ids() {
        let trace = PluginAgentTrace::try_from(RawAgentTrace {
            format: "opencode".to_owned(),
            file: Some("{data_dir}/opencode/trace/{agent_session_id}.ndjson".to_owned()),
            search: None,
        })
        .unwrap();
        let context = TraceResolveContext {
            home: Path::new("/home/user"),
            data_dir: Path::new("/home/user/.local/share"),
            agent_session_id: "../escape",
        };
        assert!(matches!(
            trace.resolve(&context),
            Err(TraceResolutionError::UnsafeSessionId { .. })
        ));
        let context = TraceResolveContext {
            agent_session_id: "",
            ..context
        };
        assert!(matches!(
            trace.resolve(&context),
            Err(TraceResolutionError::EmptySessionId)
        ));
    }
}
