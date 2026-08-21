use crate::issue::{PluginDiscoveryIssue, PluginDiscoveryIssueKind};
use ora_utils::svg::{SvgReadError, read_validated};
use std::io;
use std::path::Path;

/// The fixed package-relative filename every plugin ships its icon under.
///
/// The icon is a convention rather than a manifest field: keeping it out of `orax.toml` means a
/// package can gain or lose its icon without a schema change, and the host never has to resolve
/// an author-supplied path that could escape the package root.
const LOGO_FILE_NAME: &str = "logo.svg";

/// Reads one package's optional icon into trusted SVG source text.
///
/// `Ok(None)` means the package simply ships no icon, which is a normal state that every host
/// surface renders with its generic fallback mark. An `Err` means the file is there but cannot be
/// trusted or read, which is reported as a non-fatal discovery issue so a bad icon degrades the
/// package's presentation without hiding the plugin itself.
pub(crate) fn read(package_root: &Path) -> Result<Option<String>, PluginDiscoveryIssue> {
    let logo_path = package_root.join(LOGO_FILE_NAME);
    match read_validated(&logo_path) {
        Ok(source) => Ok(Some(source)),
        Err(SvgReadError::Unreadable(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(PluginDiscoveryIssue::new(
            logo_path,
            PluginDiscoveryIssueKind::UnusableLogo,
            None,
            error.to_string(),
        )),
    }
}
