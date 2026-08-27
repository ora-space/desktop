//! Host-side validation of the `hook` kind: a processless package whose single artifact is the
//! compiled Hook Configuration from `assets/config.json` plus one package-contained executable.

use crate::validation::{
    CONFIGURATION_FILE, INSTALLED_ENTRYPOINT, ManifestValidationError, invalid,
};
use ora_plugin_config::{
    CompileConfigurationFileError, CompiledConfigurationFile, CompiledHookConfiguration,
    HookDescriptor,
};
use ora_plugin_manifest::{HookTarget, PluginArtifact};
use ora_utils::path::CanonicalPathRoot;
use std::path::{Component, Path};

/// Holds the validated Hook descriptor of one hook-kind package.
///
/// The descriptor proves the package is statically valid. It is not a `ResolvedHook`: it says
/// nothing about a future Agent Plugin consuming it, and runnability is established by isolated
/// release and end-to-end tests, never by executing the payload during installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledHookDescriptor {
    pub configuration: CompiledHookConfiguration,
    pub artifact_target: Option<HookTarget>,
}

/// Validates one hook-kind package around its already-compiled configuration file and the
/// installed artifact self-declaration.
///
/// A Hook package must not look runnable (no `main.js`), must ship a Hook-shaped
/// `assets/config.json`, must contain the declared executable as a real regular file inside the
/// package `assets/` tree, and — for a targeted package — must self-declare a target whose host
/// compatibility is checked by the installer. Validation never executes the executable.
pub(crate) fn validate_hook(
    package_root: &Path,
    configuration_file: &Result<Option<CompiledConfigurationFile>, CompileConfigurationFileError>,
    artifact: Option<&PluginArtifact>,
) -> Result<InstalledHookDescriptor, ManifestValidationError> {
    // Same policy as mcp/webview: an entrypoint would suggest a process the host never starts and
    // would be a silent way to smuggle code into a processless kind.
    if package_root.join(INSTALLED_ENTRYPOINT).exists() {
        return Err(invalid(
            "kind",
            format!("a hook plugin must not ship `{INSTALLED_ENTRYPOINT}`"),
        ));
    }
    let configuration = match configuration_file {
        // A broken declaration rejects the whole package: the configuration file is the entire
        // contribution of a Hook package.
        Err(error) => {
            return Err(invalid(
                CONFIGURATION_FILE,
                format!("Hook configuration is invalid: {error}"),
            ));
        }
        Ok(None) => {
            return Err(invalid(
                CONFIGURATION_FILE,
                format!("a hook plugin must ship `{CONFIGURATION_FILE}`"),
            ));
        }
        // A Hook package must declare the Hook shape; a Settings-only or MCP file is a kind
        // mismatch the host rejects so a package cannot masquerade as another contribution type.
        Ok(Some(CompiledConfigurationFile::Settings(_))) => {
            return Err(invalid(
                CONFIGURATION_FILE,
                "a hook plugin must declare a `hook` contribution",
            ));
        }
        Ok(Some(CompiledConfigurationFile::Mcp(_))) => {
            return Err(invalid(
                CONFIGURATION_FILE,
                "a hook plugin must not declare an MCP `transport`",
            ));
        }
        Ok(Some(CompiledConfigurationFile::Hook(configuration))) => configuration.clone(),
    };
    validate_executable_containment(package_root, &configuration.hook)?;
    // The artifact target is carried by the installed manifest; the installer verifies it matches
    // the selected release target (online) or the current host (local import).
    Ok(InstalledHookDescriptor {
        configuration,
        artifact_target: artifact.map(|artifact| artifact.target().clone()),
    })
}

/// Confirms the compiled executable resolves to a regular non-symlink file contained under this
/// package's `assets/` tree, refusing symlink or reparse-point escapes. On Windows the milestone
/// requires the `.exe` suffix; PE headers are not parsed here.
fn validate_executable_containment(
    package_root: &Path,
    hook: &HookDescriptor,
) -> Result<(), ManifestValidationError> {
    let executable = hook.executable.as_str();
    let declared = Path::new(executable);
    if declared
        .components()
        .next()
        .is_none_or(|component| component != Component::Normal("assets".as_ref()))
    {
        return Err(invalid(
            "hook.executable",
            format!("executable `{executable}` must be contained under the package assets tree"),
        ));
    }
    let root = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "hook.executable",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let resolved = root.resolve_existing(&hook.executable).map_err(|error| {
        invalid(
            "hook.executable",
            format!("executable `{executable}` must exist inside the plugin package: {error}"),
        )
    })?;
    // The canonical check covers the current symlink target only; is_file remains path-based and
    // cannot prevent a caller-controlled replacement between validation and later use.
    if !resolved.is_file() {
        return Err(invalid(
            "hook.executable",
            format!("executable `{executable}` must be a regular package file"),
        ));
    }
    let relative = root.relative_path(&resolved).map_err(|error| {
        invalid(
            "hook.executable",
            format!("executable must resolve inside the plugin package: {error}"),
        )
    })?;
    if Path::new(relative.as_str())
        .components()
        .next()
        .is_none_or(|component| component != Component::Normal("assets".as_ref()))
    {
        return Err(invalid(
            "hook.executable",
            format!("executable `{executable}` must resolve under the package assets tree"),
        ));
    }
    // Windows decides executability by extension at spawn time; the milestone requires the `.exe`
    // suffix on the first RTK release target so an unusable binary is never installed as valid.
    // PE headers are intentionally not parsed in this milestone.
    #[cfg(windows)]
    {
        if !executable.ends_with(".exe") {
            return Err(invalid(
                "hook.executable",
                format!("executable `{executable}` must end with `.exe` on Windows"),
            ));
        }
    }
    Ok(())
}
