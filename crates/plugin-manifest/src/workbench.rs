use crate::{InvalidFieldReason, ManifestError, ManifestField};
use serde::Deserialize;
use std::{fmt, str::FromStr};
use thiserror::Error;

/// Upper bound on one method name; long enough for namespaced names, short enough to log.
const MAX_METHOD_NAME_BYTES: usize = 128;
/// The prefix of every host-defined protocol method; a plugin cannot expose one to its page.
const HOST_METHOD_PREFIX: &str = "ora/";

/// Holds the validated `[workbench]` section of a workbench-kind plugin manifest.
///
/// `methods` is the page-visible interface: the set of `main.js` methods a plugin page may call
/// through the bridge. It only narrows what the page can reach and never grants `main.js` any
/// host capability. The effective set at runtime is the intersection with what the running
/// process generation actually registered; that intersection is computed by the host.
///
/// `host_capabilities` is the one thing that does grant capability: it declares which closed-set
/// host facilities the plugin process may use. Each capability is disclosed to the user at
/// install time, and a host method that requires one refuses an undeclared caller.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginWorkbench {
    pub(crate) methods: Vec<MethodName>,
    pub(crate) host_capabilities: Vec<HostCapability>,
}

impl PluginWorkbench {
    /// Returns the page-visible methods in declaration order, without duplicates.
    pub fn methods(&self) -> &[MethodName] {
        &self.methods
    }

    /// Returns the requested host capabilities in declaration order, without duplicates.
    pub fn host_capabilities(&self) -> &[HostCapability] {
        &self.host_capabilities
    }
}

/// Identifies the closed set of host capabilities a workbench plugin may request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HostCapability {
    /// `session.trace`: read the trace files of agent sessions through the host's
    /// `ora/session/trace_*` methods, never a filesystem path.
    SessionTrace,
}

impl HostCapability {
    /// Returns the manifest spelling of this capability.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionTrace => "session.trace",
        }
    }
}

impl fmt::Display for HostCapability {
    /// Writes the manifest spelling of this capability.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for HostCapability {
    type Err = HostCapabilityError;

    /// Parses a capability without accepting future values under resolver version 1.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "session.trace" => Ok(Self::SessionTrace),
            found => Err(HostCapabilityError::Unsupported {
                found: found.to_owned(),
            }),
        }
    }
}

/// Reports an unsupported host capability spelling.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HostCapabilityError {
    #[error("unsupported host capability {found:?}")]
    Unsupported { found: String },
}

/// A plugin protocol method name a page may invoke: `segment(/segment)*` of `[a-z0-9_]`.
///
/// The `ora/` namespace is reserved for host-defined protocol methods and is rejected here so a
/// manifest cannot expose a host method (such as `ora/storage/read`) to an untrusted page.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MethodName(String);

impl MethodName {
    /// Parses one method name without normalizing it; the spelling is the identity.
    pub fn parse(value: &str) -> Result<Self, MethodNameError> {
        if value.is_empty() {
            return Err(MethodNameError::Empty);
        }
        if value.len() > MAX_METHOD_NAME_BYTES {
            return Err(MethodNameError::TooLong {
                max_bytes: MAX_METHOD_NAME_BYTES,
                actual_bytes: value.len(),
            });
        }
        if value.starts_with(HOST_METHOD_PREFIX) {
            return Err(MethodNameError::ReservedPrefix);
        }
        for segment in value.split('/') {
            if segment.is_empty() {
                return Err(MethodNameError::EmptySegment);
            }
            if let Some(character) = segment
                .chars()
                .find(|character| !matches!(character, 'a'..='z' | '0'..='9' | '_'))
            {
                return Err(MethodNameError::InvalidCharacter { character });
            }
        }

        Ok(Self(value.to_owned()))
    }

    /// Returns the method name as spelled in the manifest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MethodName {
    /// Writes the method name verbatim.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for MethodName {
    type Err = MethodNameError;

    /// Parses a method name through the same rules as `parse`.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Reports why a method name was rejected.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MethodNameError {
    #[error("method name must not be empty")]
    Empty,
    #[error("method name exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("method name must not use the reserved `ora/` host namespace")]
    ReservedPrefix,
    #[error("method name must not contain an empty segment")]
    EmptySegment,
    #[error("method name contains invalid character {character:?}")]
    InvalidCharacter { character: char },
}

/// Mirrors `[workbench]` before semantic validation; unknown fields fail structurally.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawWorkbench {
    methods: Vec<String>,
    host_capabilities: Option<Vec<String>>,
}

impl TryFrom<RawWorkbench> for PluginWorkbench {
    type Error = ManifestError;

    /// Validates every method name in declaration order and rejects duplicates.
    ///
    /// An empty list is rejected rather than accepted as "no methods": a plugin that wants a
    /// purely static page omits the section instead of declaring an empty allowlist.
    fn try_from(raw: RawWorkbench) -> Result<Self, Self::Error> {
        if raw.methods.is_empty() {
            return Err(ManifestError::InvalidField {
                field: ManifestField::WorkbenchMethods,
                reason: InvalidFieldReason::Empty,
            });
        }
        let mut methods = Vec::with_capacity(raw.methods.len());
        for (index, value) in raw.methods.iter().enumerate() {
            let method =
                MethodName::parse(value).map_err(|reason| ManifestError::InvalidField {
                    field: ManifestField::WorkbenchMethod { index },
                    reason: reason.into(),
                })?;
            if methods.contains(&method) {
                return Err(ManifestError::InvalidField {
                    field: ManifestField::WorkbenchMethod { index },
                    reason: InvalidFieldReason::Duplicate,
                });
            }
            methods.push(method);
        }

        let mut host_capabilities = Vec::new();
        for (index, value) in raw.host_capabilities.unwrap_or_default().iter().enumerate() {
            let capability =
                HostCapability::from_str(value).map_err(|reason| ManifestError::InvalidField {
                    field: ManifestField::WorkbenchHostCapability { index },
                    reason: reason.into(),
                })?;
            if host_capabilities.contains(&capability) {
                return Err(ManifestError::InvalidField {
                    field: ManifestField::WorkbenchHostCapability { index },
                    reason: InvalidFieldReason::Duplicate,
                });
            }
            host_capabilities.push(capability);
        }

        Ok(Self {
            methods,
            host_capabilities,
        })
    }
}
