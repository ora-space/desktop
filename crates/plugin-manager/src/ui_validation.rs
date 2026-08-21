use crate::manifest::{
    InstancePolicyManifest, NavigationManifest, SurfaceManifest, SurfaceSourceManifest, UiManifest,
    WebDataPolicyManifest,
};
use crate::surface::{HostName, InstancePolicy, SurfaceId, WebDataPolicy};
use crate::validation::{ManifestValidationError, invalid};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use url::Url;

const SUPPORTED_UI_CONTRACT_VERSION: u32 = 1;
/// Surfaces per package are bounded because each one becomes a menu entry and a potential
/// webview; an unbounded list would let one package flood the host UI.
const MIN_SURFACES: usize = 1;
const MAX_SURFACES: usize = 8;
/// Titles are rendered in menus and tab strips, so they are kept short.
const MAX_TITLE_CHARS: usize = 64;
/// A panel entry is loaded as a document, so only HTML makes sense as the first request.
const PANEL_ENTRY_EXTENSION: &str = "html";

/// Holds the validated ui contribution of one ui-kind package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPluginUi {
    pub contract_version: u32,
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

/// Validates `ora.contributes.ui` field by field, reporting the first failing field path.
///
/// `package_root` is needed because a panel declaration points at files that must exist inside
/// the package; a manifest that names a missing page is rejected at discovery, not when the
/// surface is opened.
pub(crate) fn validate_ui(
    package_root: &Path,
    ui: UiManifest,
) -> Result<InstalledPluginUi, ManifestValidationError> {
    if ui.contract_version != SUPPORTED_UI_CONTRACT_VERSION {
        return Err(invalid(
            "ora.contributes.ui.contractVersion",
            format!(
                "unsupported ui contract version {}; expected {SUPPORTED_UI_CONTRACT_VERSION}",
                ui.contract_version
            ),
        ));
    }
    let count = ui.surfaces.len();
    if !(MIN_SURFACES..=MAX_SURFACES).contains(&count) {
        return Err(invalid(
            "ora.contributes.ui.surfaces",
            format!(
                "a ui plugin must declare between {MIN_SURFACES} and {MAX_SURFACES} surfaces; found {count}"
            ),
        ));
    }

    // A BTreeMap keyed by id gives uniqueness detection and the sorted output in one pass.
    let mut surfaces = BTreeMap::new();
    for (index, surface) in ui.surfaces.into_iter().enumerate() {
        let prefix = format!("ora.contributes.ui.surfaces[{index}]");
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
        contract_version: ui.contract_version,
        surfaces: surfaces.into_values().collect(),
    })
}

/// Validates one surface entry whose field paths start with `prefix`.
fn validate_surface(
    package_root: &Path,
    prefix: &str,
    surface: SurfaceManifest,
) -> Result<InstalledSurface, ManifestValidationError> {
    let id = SurfaceId::parse(&surface.id).map_err(|error| {
        invalid(
            format!("{prefix}.id"),
            format!("invalid surface id: {error}"),
        )
    })?;
    let title = surface.title.trim();
    if title.is_empty() {
        return Err(invalid(
            format!("{prefix}.title"),
            "value must not be empty",
        ));
    }
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
    let instance_policy = match surface.instance_policy {
        InstancePolicyManifest::Singleton => InstancePolicy::Singleton,
    };
    let source = match surface.source {
        SurfaceSourceManifest::RemoteSite {
            entry_url,
            navigation,
            web_data,
        } => InstalledSurfaceSource::RemoteSite(validate_remote_site(
            &format!("{prefix}.source"),
            &entry_url,
            navigation,
            web_data,
        )?),
        SurfaceSourceManifest::Panel { root, entry } => InstalledSurfaceSource::Panel(
            validate_panel(&format!("{prefix}.source"), package_root, &root, &entry)?,
        ),
    };

    Ok(InstalledSurface {
        id,
        title: title.to_owned(),
        instance_policy,
        source,
    })
}

/// Validates the entry URL and the navigation policy that must contain it.
fn validate_remote_site(
    prefix: &str,
    entry_url: &str,
    navigation: NavigationManifest,
    web_data: WebDataPolicyManifest,
) -> Result<RemoteSiteSource, ManifestValidationError> {
    let entry_field = format!("{prefix}.entryUrl");
    let entry_url = Url::parse(entry_url).map_err(|error| {
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

    let allow_hosts = validate_hosts(
        &format!("{prefix}.navigation.allowHosts"),
        navigation.allow_hosts,
    )?;
    let allow_host_suffixes = validate_hosts(
        &format!("{prefix}.navigation.allowHostSuffixes"),
        navigation.allow_host_suffixes,
    )?;
    if allow_hosts.is_empty() && allow_host_suffixes.is_empty() {
        return Err(invalid(
            format!("{prefix}.navigation"),
            "navigation must allow at least one host or host suffix",
        ));
    }
    let entry_allowed = allow_hosts.iter().any(|host| host.as_str() == entry_host)
        || allow_host_suffixes
            .iter()
            .any(|suffix| suffix.matches_suffix_of(entry_host));
    if !entry_allowed {
        return Err(invalid(
            entry_field,
            format!("entry URL host `{entry_host}` is not covered by the navigation allow lists"),
        ));
    }

    Ok(RemoteSiteSource {
        entry_url,
        allow_hosts,
        allow_host_suffixes,
        web_data: match web_data {
            WebDataPolicyManifest::PersistentProfile => WebDataPolicy::PersistentProfile,
            WebDataPolicyManifest::EphemeralIsolated => WebDataPolicy::EphemeralIsolated,
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
    // Serving the package root would expose `package.json` and the plugin source to the page.
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
    hosts: Vec<String>,
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
