use ora_utils::archive::ExtractLimits;

/// Cap on the `.orax` archive accepted before extraction begins.
const MAX_PACKAGE_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

/// Cap on the cumulative bytes one package may materialize on disk.
const MAX_PACKAGE_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

/// Cap on the workflow documents one Workflow plugin package may contribute.
///
/// The extract budget already bounds the bytes, so this bounds the work instead: importing a
/// package reads, validates, and persists every document it carries, and the import response
/// reports each one. A workflow package is a delivery vehicle for user data rather than a
/// program, so the ceiling is set by what one intentional import should be able to carry in a
/// single step, not by what a package could physically hold.
pub(crate) const MAX_WORKFLOWS_PER_PACKAGE: usize = 256;

/// Returns the extraction limits applied to every plugin package.
///
/// A plugin package is not a document bundle. An Agent Plugin may ship the CLI it drives so the
/// user does not have to install one, and those are single native binaries in the hundreds of
/// megabytes — OpenCode is roughly 176 MiB unpacked from a 58 MiB archive. `ExtractLimits`'s
/// generic default is sized for text packages and would refuse such a release before extraction
/// ever started, so plugin installs declare their own budget here rather than inheriting one
/// tuned for a different kind of payload.
///
/// The entry-count and path limits stay at the shared defaults: shipping a large binary is a
/// reason to raise the byte budgets, not to accept deeper trees or more files.
pub(crate) fn package_extract_limits() -> ExtractLimits {
    ExtractLimits {
        max_archive_bytes: MAX_PACKAGE_ARCHIVE_BYTES,
        max_total_bytes: MAX_PACKAGE_TOTAL_BYTES,
        ..ExtractLimits::default()
    }
}
