use crate::validation::{INSTALLED_ENTRYPOINT, ManifestValidationError, invalid};
use ora_plugin_manifest::{DownloadPolicy, Origin, PluginWebview, StartUrl};
use std::collections::BTreeSet;
use std::path::Path;

/// Holds the validated contribution of one webview-kind package.
///
/// A webview plugin is configuration only: there is no process and no entrypoint. The
/// descriptor is the immutable snapshot an open instance binds to, so an upgrade never changes
/// the navigation or download rules of a page the user is already looking at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledWebviewDescriptor {
    pub start_url: StartUrl,
    /// Manifest order, duplicates rejected at validation; always covers `start_url`.
    pub allowed_origins: Vec<Origin>,
    pub download_policy: DownloadPolicy,
}

/// Applies the host's webview policy to a parsed manifest, reporting the first failing field.
///
/// The manifest crate already checked each value's syntax; this layer checks what needs the
/// whole declaration or the package on disk: no runnable entrypoint, no duplicate origin, the
/// start URL inside the origin set, and no download rule that can never fire.
pub(crate) fn validate_webview(
    package_root: &Path,
    webview: &PluginWebview,
) -> Result<InstalledWebviewDescriptor, ManifestValidationError> {
    // A webview package must not look runnable: an entrypoint would suggest a process the host
    // will never start, and would be a silent way to smuggle code into a config-only kind.
    if package_root.join(INSTALLED_ENTRYPOINT).exists() {
        return Err(invalid(
            "kind",
            format!("a webview plugin must not ship `{INSTALLED_ENTRYPOINT}`"),
        ));
    }

    let mut seen = BTreeSet::new();
    for (index, origin) in webview.allowed_origins().iter().enumerate() {
        if !seen.insert(origin) {
            return Err(invalid(
                format!("webview.allowed_origins[{index}]"),
                format!("origin `{origin}` is declared more than once"),
            ));
        }
    }
    let start_origin = webview.start_url().origin();
    if !seen.contains(&start_origin) {
        return Err(invalid(
            "webview.start_url",
            format!("start URL origin `{start_origin}` is not in `allowed_origins`"),
        ));
    }

    let policy = webview.downloads();
    for (index, rule) in policy.rules.iter().enumerate() {
        if !seen.contains(&rule.page.origin) {
            return Err(invalid(
                format!("webview.downloads.rules[{index}].page.origin"),
                format!(
                    "rule origin `{}` is not in `allowed_origins`",
                    rule.page.origin
                ),
            ));
        }
        // First match wins, so a rule whose page set is covered by an earlier rule on the same
        // origin can never fire; rejecting it keeps authors from believing it does.
        if let Some(shadow) = policy.rules[..index].iter().position(|earlier| {
            earlier.page.origin == rule.page.origin
                && rule
                    .page
                    .path_prefix
                    .as_str()
                    .starts_with(earlier.page.path_prefix.as_str())
        }) {
            return Err(invalid(
                format!("webview.downloads.rules[{index}]"),
                format!("rule is shadowed by rule {shadow} and can never match"),
            ));
        }
    }

    Ok(InstalledWebviewDescriptor {
        start_url: webview.start_url().clone(),
        allowed_origins: webview.allowed_origins().to_vec(),
        download_policy: policy.clone(),
    })
}
