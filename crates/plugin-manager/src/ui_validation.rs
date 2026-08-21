use crate::surface::{HostName, InstancePolicy, SurfaceId, WebDataPolicy};
use crate::validation::{ManifestValidationError, invalid};
use ora_plugin_manifest::{
    PluginUi, SurfaceDeclaration, SurfaceInstances, SurfaceSource, WebDataMode,
};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use url::Url;

/// Surfaces per package are bounded because each one becomes a menu entry and a potential
/// webview; an unbounded list would let one package flood the host UI.
const MAX_SURFACES: usize = 8;
/// Titles are rendered in menus and tab strips, so they are kept short.
const MAX_TITLE_CHARS: usize = 64;
/// A panel entry is loaded as a document, so only HTML makes sense as the first request.
const PANEL_ENTRY_EXTENSION: &str = "html";

/// Holds the validated ui contribution of one ui-kind package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPluginUi {
    /// Deduplicated and sorted by id so snapshot order never depends on manifest order.
    pub surfaces: Vec<InstalledSurface>,
}

/// Holds one validated surface declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSurface {
    pub id: SurfaceId,
    pub title: String,
    pub instance_policy: InstancePolicy,
    pub source: InstalledSurfaceSource,
}

/// Holds the validated content source of one surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledSurfaceSource {
    RemoteSite(RemoteSiteSource),
    Panel(PanelSource),
}

/// Holds one validated remote site: an https entry plus the navigation policy that contains it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSiteSource {
    pub entry_url: Url,
    pub allow_hosts: Vec<HostName>,
    pub allow_host_suffixes: Vec<HostName>,
    pub web_data: WebDataPolicy,
}

/// Holds one validated panel: the canonical asset directory inside the package and the entry
/// document relative to it. Only files below `asset_root` are ever served to the panel webview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSource {
    pub asset_root: PathBuf,
    pub entry: PortableRelativePath,
}

/// Applies the host's surface policy to a structurally valid `[ui]` section, reporting the
/// first failing field path.
///
/// `package_root` is needed because a panel declaration points at files that must exist inside
/// the package; a manifest that names a missing page is rejected at discovery, not when the
/// surface is opened.
pub(crate) fn validate_ui(
    package_root: &Path,
    ui: &PluginUi,
) -> Result<InstalledPluginUi, ManifestValidationError> {
    let count = ui.surfaces().len();
    if count > MAX_SURFACES {
        return Err(invalid(
            "ui.surfaces",
            format!("a ui plugin may declare at most {MAX_SURFACES} surfaces; found {count}"),
        ));
    }

    // A BTreeMap keyed by id gives uniqueness detection and the sorted output in one pass.
    let mut surfaces = BTreeMap::new();
    for (index, surface) in ui.surfaces().iter().enumerate() {
        let prefix = format!("ui.surfaces[{index}]");
        let surface = validate_surface(package_root, &prefix, surface)?;
        if surfaces.contains_key(&surface.id) {
            return Err(invalid(
                format!("{prefix}.id"),
                format!("duplicate surface id `{}`", surface.id),
            ));
        }
        surfaces.insert(surface.id.clone(), surface);
    }

    Ok(InstalledPluginUi {
        surfaces: surfaces.into_values().collect(),
    })
}

/// Validates one surface entry whose field paths start with `prefix`.
fn validate_surface(
    package_root: &Path,
    prefix: &str,
    surface: &SurfaceDeclaration,
) -> Result<InstalledSurface, ManifestValidationError> {
    let id = SurfaceId::parse(surface.id().as_str()).map_err(|error| {
        invalid(
            format!("{prefix}.id"),
            format!("invalid surface id: {error}"),
        )
    })?;
    let title = surface.title().trim();
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid(
            format!("{prefix}.title"),
            format!("surface title exceeds {MAX_TITLE_CHARS} characters"),
        ));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(
            format!("{prefix}.title"),
            "surface title must not contain control characters",
        ));
    }
    // The registry and the frontend panel slot only model one live instance per definition;
    // accepting `multiple` here would silently degrade to singleton, so it is refused until the
    // host can honour it.
    let instance_policy = match surface.instances() {
        SurfaceInstances::Singleton => InstancePolicy::Singleton,
        SurfaceInstances::Multiple => {
            return Err(invalid(
                format!("{prefix}.instances"),
                "`multiple` instances are not supported yet; declare `singleton` or omit the field",
            ));
        }
    };
    let source = match surface.source() {
        SurfaceSource::RemoteSite {
            entry,
            hosts,
            host_suffixes,
        } => InstalledSurfaceSource::RemoteSite(validate_remote_site(
            prefix,
            entry,
            hosts,
            host_suffixes,
            surface.web_data(),
        )?),
        SurfaceSource::Panel { root, entry } => {
            // Panels always get an isolated persistent profile of their own; a declared policy
            // would either be redundant or contradict that guarantee.
            if surface.web_data().is_some() {
                return Err(invalid(
                    format!("{prefix}.web_data"),
                    "panel surfaces always use an isolated persistent profile; remove `web_data`",
                ));
            }
            InstalledSurfaceSource::Panel(validate_panel(
                &format!("{prefix}.source"),
                package_root,
                root,
                entry,
            )?)
        }
    };

    Ok(InstalledSurface {
        id,
        title: title.to_owned(),
        instance_policy,
        source,
    })
}

/// Validates the entry URL and the navigation allow lists that must contain it.
fn validate_remote_site(
    prefix: &str,
    entry: &str,
    hosts: &[String],
    host_suffixes: &[String],
    web_data: Option<WebDataMode>,
) -> Result<RemoteSiteSource, ManifestValidationError> {
    let entry_field = format!("{prefix}.source.entry");
    let entry_url = Url::parse(entry).map_err(|error| {
        invalid(
            entry_field.clone(),
            format!("entry URL must be an absolute URL: {error}"),
        )
    })?;
    if entry_url.scheme() != "https" {
        return Err(invalid(entry_field, "entry URL scheme must be `https`"));
    }
    if !entry_url.username().is_empty() || entry_url.password().is_some() {
        return Err(invalid(
            entry_field,
            "entry URL must not carry a username or password",
        ));
    }
    if entry_url.port().is_some() {
        return Err(invalid(entry_field, "entry URL must not specify a port"));
    }
    let Some(entry_host) = entry_url.host_str().filter(|host| !host.is_empty()) else {
        return Err(invalid(entry_field, "entry URL must have a host"));
    };

    let allow_hosts = validate_hosts(&format!("{prefix}.source.hosts"), hosts)?;
    let allow_host_suffixes =
        validate_hosts(&format!("{prefix}.source.host_suffixes"), host_suffixes)?;
    if allow_hosts.is_empty() && allow_host_suffixes.is_empty() {
        return Err(invalid(
            format!("{prefix}.source"),
            "a remote site must allow at least one host or host suffix",
        ));
    }
    let entry_allowed = allow_hosts.iter().any(|host| host.as_str() == entry_host)
        || allow_host_suffixes
            .iter()
            .any(|suffix| suffix.matches_suffix_of(entry_host));
    if !entry_allowed {
        return Err(invalid(
            entry_field,
            format!("entry URL host `{entry_host}` is not covered by `hosts` or `host_suffixes`"),
        ));
    }

    Ok(RemoteSiteSource {
        entry_url,
        allow_hosts,
        allow_host_suffixes,
        web_data: match web_data {
            Some(WebDataMode::Persistent) | None => WebDataPolicy::PersistentProfile,
            Some(WebDataMode::Ephemeral) => WebDataPolicy::EphemeralIsolated,
        },
    })
}

/// Resolves the panel asset directory and entry document, both of which must exist inside the
/// package. The directory is canonicalized once here so the asset handler can treat it as a
/// containment root without repeating the package-boundary reasoning.
fn validate_panel(
    prefix: &str,
    package_root: &Path,
    root: &str,
    entry: &str,
) -> Result<PanelSource, ManifestValidationError> {
    let root_field = format!("{prefix}.root");
    let root_relative = PortableRelativePath::parse(root).map_err(|error| {
        invalid(
            root_field.clone(),
            format!("panel root must be a safe relative path: {error}"),
        )
    })?;
    // Serving the package root would expose `orax.toml` and the plugin source to the page.
    if root_relative.is_root() {
        return Err(invalid(
            root_field,
            "panel root must be a subdirectory of the package",
        ));
    }
    let package = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            root_field.clone(),
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let asset_root = package.resolve_existing(&root_relative).map_err(|error| {
        invalid(
            root_field.clone(),
            format!("panel root must resolve inside the plugin package: {error}"),
        )
    })?;
    if !asset_root.is_dir() {
        return Err(invalid(root_field, "panel root must be a directory"));
    }

    let entry_field = format!("{prefix}.entry");
    let entry_relative = PortableRelativePath::parse(entry).map_err(|error| {
        invalid(
            entry_field.clone(),
            format!("panel entry must be a safe relative path: {error}"),
        )
    })?;
    if entry_relative.is_root() {
        return Err(invalid(entry_field, "panel entry must identify a file"));
    }
    if entry_relative
        .to_path_buf()
        .extension()
        .and_then(|ext| ext.to_str())
        != Some(PANEL_ENTRY_EXTENSION)
    {
        return Err(invalid(
            entry_field,
            format!("panel entry must be an `.{PANEL_ENTRY_EXTENSION}` document"),
        ));
    }
    let assets = CanonicalPathRoot::new(&asset_root).map_err(|error| {
        invalid(
            entry_field.clone(),
            format!("panel root is unavailable: {error}"),
        )
    })?;
    let entry_path = assets.resolve_existing(&entry_relative).map_err(|error| {
        invalid(
            entry_field.clone(),
            format!("panel entry must resolve inside the panel root: {error}"),
        )
    })?;
    if !entry_path.is_file() {
        return Err(invalid(
            entry_field,
            "panel entry must identify a regular file",
        ));
    }

    Ok(PanelSource {
        asset_root,
        entry: entry_relative,
    })
}

/// Parses one allow list, reporting the offending index on failure.
fn validate_hosts(
    prefix: &str,
    hosts: &[String],
) -> Result<Vec<HostName>, ManifestValidationError> {
    hosts
        .iter()
        .enumerate()
        .map(|(index, host)| {
            HostName::parse(host).map_err(|error| {
                invalid(
                    format!("{prefix}[{index}]"),
                    format!("invalid host: {error}"),
                )
            })
        })
        .collect()
}
