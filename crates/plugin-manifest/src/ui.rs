use crate::{InvalidFieldReason, ManifestError, ManifestField, SurfaceField};
use ora_utils::Slug;
use serde::Deserialize;
use std::{fmt, str::FromStr};
use thiserror::Error;

/// Holds the validated `[ui]` section of a ui-kind plugin manifest.
///
/// The manifest crate guarantees only the declaration's shape: every surface has a slug id, a
/// non-empty title, known enum spellings, and the fields its source kind requires. Host-specific
/// policy (URL scheme, allow-list coverage, panel files on disk, surface limits) is applied by
/// the plugin manager, which owns the package on disk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginUi {
    pub(crate) surfaces: Vec<SurfaceDeclaration>,
}

impl PluginUi {
    /// Returns the declared surfaces in manifest order; there is always at least one.
    pub fn surfaces(&self) -> &[SurfaceDeclaration] {
        &self.surfaces
    }
}

/// Holds one structurally validated `[[ui.surfaces]]` entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceDeclaration {
    pub(crate) id: Slug,
    pub(crate) title: String,
    pub(crate) instances: SurfaceInstances,
    pub(crate) source: SurfaceSource,
    pub(crate) web_data: Option<WebDataMode>,
}

impl SurfaceDeclaration {
    /// Returns the surface id; uniqueness within the plugin is checked by the manager.
    pub fn id(&self) -> &Slug {
        &self.id
    }

    /// Returns the user-visible title exactly as declared.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns how many live instances the surface asks for.
    pub fn instances(&self) -> SurfaceInstances {
        self.instances
    }

    /// Returns where the surface loads its content from.
    pub fn source(&self) -> &SurfaceSource {
        &self.source
    }

    /// Returns the declared web data mode, or `None` when the manifest left it implicit.
    ///
    /// The default is left to the caller because whether a declaration is even allowed depends
    /// on the source kind (panels own their profile), which only the manager decides.
    pub fn web_data(&self) -> Option<WebDataMode> {
        self.web_data
    }
}

/// Declares how many live instances of one surface may exist; `singleton` when omitted.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum SurfaceInstances {
    #[default]
    Singleton,
    Multiple,
}

impl SurfaceInstances {
    /// Returns the manifest spelling of this policy.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Singleton => "singleton",
            Self::Multiple => "multiple",
        }
    }
}

impl fmt::Display for SurfaceInstances {
    /// Writes the manifest spelling of this policy.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SurfaceInstances {
    type Err = SurfaceInstancesError;

    /// Parses the policy without accepting unknown spellings.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "singleton" => Ok(Self::Singleton),
            "multiple" => Ok(Self::Multiple),
            found => Err(SurfaceInstancesError::Unsupported {
                found: found.to_owned(),
            }),
        }
    }
}

/// Reports an unsupported `instances` spelling.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SurfaceInstancesError {
    #[error("unsupported surface instances policy {found:?}")]
    Unsupported { found: String },
}

/// Declares where the web data (cookies, storage) of one surface lives.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WebDataMode {
    Persistent,
    Ephemeral,
}

impl WebDataMode {
    /// Returns the manifest spelling of this mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::Ephemeral => "ephemeral",
        }
    }
}

impl fmt::Display for WebDataMode {
    /// Writes the manifest spelling of this mode.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for WebDataMode {
    type Err = WebDataModeError;

    /// Parses the mode without accepting unknown spellings.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "persistent" => Ok(Self::Persistent),
            "ephemeral" => Ok(Self::Ephemeral),
            found => Err(WebDataModeError::Unsupported {
                found: found.to_owned(),
            }),
        }
    }
}

/// Reports an unsupported `web_data.mode` spelling.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum WebDataModeError {
    #[error("unsupported web data mode {found:?}")]
    Unsupported { found: String },
}

/// Describes the content source of one surface, discriminated by `source.kind`.
///
/// Strings are kept verbatim: the entry URL and host lists of a remote site, and the package
/// paths of a panel, are interpreted against host policy and the package directory by the
/// plugin manager, not here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SurfaceSource {
    RemoteSite {
        entry: String,
        hosts: Vec<String>,
        host_suffixes: Vec<String>,
    },
    Panel {
        root: String,
        entry: String,
    },
}

/// Mirrors `[ui]` before semantic validation; unknown fields fail structurally.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawUi {
    surfaces: Vec<RawSurface>,
}

/// Mirrors one `[[ui.surfaces]]` entry before semantic validation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawSurface {
    id: String,
    title: String,
    instances: Option<String>,
    source: RawSurfaceSource,
    web_data: Option<RawWebData>,
}

/// Mirrors the tagged `[ui.surfaces.source]` union.
///
/// An unknown `kind`, or a field that belongs to the other form, fails during deserialization so
/// the error carries the offending TOML path instead of a semantic field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawSurfaceSource {
    RemoteSite {
        entry: String,
        #[serde(default)]
        hosts: Vec<String>,
        #[serde(default)]
        host_suffixes: Vec<String>,
    },
    Panel {
        root: String,
        entry: String,
    },
}

/// Mirrors `[ui.surfaces.web_data]`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawWebData {
    mode: String,
}

impl TryFrom<RawUi> for PluginUi {
    type Error = ManifestError;

    /// Validates every surface in declaration order so the first error is deterministic.
    fn try_from(raw: RawUi) -> Result<Self, Self::Error> {
        if raw.surfaces.is_empty() {
            return Err(ManifestError::InvalidField {
                field: ManifestField::UiSurfaces,
                reason: InvalidFieldReason::Empty,
            });
        }
        let surfaces = raw
            .surfaces
            .into_iter()
            .enumerate()
            .map(|(index, surface)| validate_surface(index, surface))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { surfaces })
    }
}

/// Applies the shape rules of one surface, attributing failures to its indexed field.
fn validate_surface(index: usize, raw: RawSurface) -> Result<SurfaceDeclaration, ManifestError> {
    let invalid = |field: SurfaceField, reason: InvalidFieldReason| ManifestError::InvalidField {
        field: ManifestField::UiSurface { index, field },
        reason,
    };
    let id = Slug::parse(&raw.id).map_err(|reason| invalid(SurfaceField::Id, reason.into()))?;
    if raw.title.trim().is_empty() {
        return Err(invalid(SurfaceField::Title, InvalidFieldReason::Empty));
    }
    let instances = raw
        .instances
        .as_deref()
        .map(SurfaceInstances::from_str)
        .transpose()
        .map_err(|reason| invalid(SurfaceField::Instances, reason.into()))?
        .unwrap_or_default();
    let source = match raw.source {
        RawSurfaceSource::RemoteSite {
            entry,
            hosts,
            host_suffixes,
        } => SurfaceSource::RemoteSite {
            entry,
            hosts,
            host_suffixes,
        },
        RawSurfaceSource::Panel { root, entry } => SurfaceSource::Panel { root, entry },
    };
    let web_data = raw
        .web_data
        .map(|web_data| WebDataMode::from_str(&web_data.mode))
        .transpose()
        .map_err(|reason| invalid(SurfaceField::WebDataMode, reason.into()))?;

    Ok(SurfaceDeclaration {
        id,
        title: raw.title,
        instances,
        source,
        web_data,
    })
}
