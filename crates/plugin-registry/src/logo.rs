use ora_logging::ora_warn;
use ora_utils::svg::{SvgReadError, read_validated};
use std::io;
use std::path::Path;

/// The fixed filename every marketplace registry entry ships its icon under, beside `orax.toml`.
///
/// The icon is a directory convention rather than a manifest field, so a registry entry can gain
/// or lose its icon without a schema change and the index build never resolves an author-supplied
/// path.
const LOGO_FILE_NAME: &str = "logo.svg";

/// Reads the optional icon that sits beside one registry manifest into trusted SVG source text.
///
/// The index is a derived artifact built from many entries, so an entry whose icon is missing,
/// unreadable, or unsafe still gets indexed without one: an icon problem must never remove a
/// plugin from the marketplace listing. Unsafe and unreadable icons are logged, while an absent
/// icon is the ordinary case and stays silent.
pub(crate) fn read_beside_manifest(manifest_path: &Path) -> Option<String> {
    let logo_path = manifest_path.parent()?.join(LOGO_FILE_NAME);
    match read_validated(&logo_path) {
        Ok(source) => Some(source),
        Err(SvgReadError::Unreadable(error)) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            ora_warn!(path = %logo_path.display(), %error, "skipping unusable registry plugin logo");
            None
        }
    }
}
