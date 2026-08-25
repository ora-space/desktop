use crate::validation::{ManifestValidationError, invalid, validate_entrypoint};
use ora_plugin_manifest::{MethodName, PluginWorkbench};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use std::path::{Path, PathBuf};

/// The package directory that holds a workbench plugin's page; nothing outside it is served.
pub const WORKBENCH_ASSET_DIRECTORY: &str = "assets";
/// The fixed page entry inside the asset directory; there is no configurable entry in v1.
pub const WORKBENCH_PAGE_ENTRY: &str = "index.html";

/// Holds the validated contribution of one workbench-kind package.
///
/// This is the immutable descriptor a surface instance binds to when it opens: the process
/// entrypoint, the canonical asset directory the resource adapter uses as its containment root,
/// the page entry below it, and the methods the manifest exposes to the page. A running surface
/// keeps its own copy so an upgrade never changes what an already open page may load or call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledWorkbenchDescriptor {
    /// `main.js` relative to the package root.
    pub entrypoint: PortableRelativePath,
    /// Canonical absolute path of `assets/`; the only directory ever served to the page.
    pub asset_root: PathBuf,
    /// `index.html` relative to `asset_root`.
    pub page_entry: PortableRelativePath,
    /// Page-visible methods in manifest order, without duplicates; empty for a static page.
    pub declared_methods: Vec<MethodName>,
}

/// Applies the host's workbench policy to a parsed manifest, reporting the first failing field.
///
/// `package_root` is needed because a workbench package must ship its process entrypoint and
/// its page: a manifest naming a missing page is rejected at discovery, not when the surface is
/// opened.
pub(crate) fn validate_workbench(
    package_root: &Path,
    workbench: Option<&PluginWorkbench>,
) -> Result<InstalledWorkbenchDescriptor, ManifestValidationError> {
    let entrypoint = validate_entrypoint(package_root)?;
    let asset_relative =
        PortableRelativePath::parse(WORKBENCH_ASSET_DIRECTORY).map_err(|error| {
            invalid(
                "workbench",
                format!("workbench asset directory name is invalid: {error}"),
            )
        })?;
    let package = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "workbench",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let asset_root = package.resolve_existing(&asset_relative).map_err(|error| {
        invalid(
            "workbench",
            format!("workbench package must ship an `{WORKBENCH_ASSET_DIRECTORY}/` directory inside the package: {error}"),
        )
    })?;
    if !asset_root.is_dir() {
        return Err(invalid(
            "workbench",
            format!("`{WORKBENCH_ASSET_DIRECTORY}` must be a directory"),
        ));
    }

    let page_entry = PortableRelativePath::parse(WORKBENCH_PAGE_ENTRY).map_err(|error| {
        invalid(
            "workbench",
            format!("workbench page entry is invalid: {error}"),
        )
    })?;
    let assets = CanonicalPathRoot::new(&asset_root).map_err(|error| {
        invalid(
            "workbench",
            format!("workbench asset directory is unavailable: {error}"),
        )
    })?;
    let page = assets.resolve_existing(&page_entry).map_err(|error| {
        invalid(
            "workbench",
            format!("workbench package must ship `{WORKBENCH_ASSET_DIRECTORY}/{WORKBENCH_PAGE_ENTRY}` inside the package: {error}"),
        )
    })?;
    // The canonical check covers the current symlink target only; is_file remains path-based
    // and cannot prevent a replacement between validation and serving.
    if !page.is_file() {
        return Err(invalid(
            "workbench",
            format!("`{WORKBENCH_ASSET_DIRECTORY}/{WORKBENCH_PAGE_ENTRY}` must be a regular file"),
        ));
    }

    Ok(InstalledWorkbenchDescriptor {
        entrypoint,
        asset_root,
        page_entry,
        declared_methods: workbench
            .map(|workbench| workbench.methods().to_vec())
            .unwrap_or_default(),
    })
}
