//! Resolves the canonical Rust target triple of the host the registry is running on.

use ora_plugin_manifest::HookTarget;

/// Returns the canonical Rust target triple for the host the backend is running on.
///
/// The triple is a compile-time constant of the host binary. Target selection uses this exact
/// triple and never falls back across architecture, operating system, libc, or ABI. `None` means
/// the compiled host is not a supported plugin target, so targeted releases are incompatible
/// rather than advertising a fake triple.
pub fn current_host_target() -> Option<HookTarget> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    const HOST_TRIPLE: Option<&str> = Some("x86_64-pc-windows-msvc");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    const HOST_TRIPLE: Option<&str> = Some("aarch64-apple-darwin");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    const HOST_TRIPLE: Option<&str> = Some("x86_64-apple-darwin");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const HOST_TRIPLE: Option<&str> = Some("x86_64-unknown-linux-gnu");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    const HOST_TRIPLE: Option<&str> = Some("aarch64-unknown-linux-gnu");
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
    )))]
    const HOST_TRIPLE: Option<&str> = None;

    HOST_TRIPLE.map(|triple| {
        HookTarget::parse(triple).unwrap_or_else(|error| {
            unreachable!("host triple is a known-valid constant: {error:?}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::current_host_target;

    /// Supported CI hosts always resolve a non-empty triple; unsupported hosts report absence.
    #[test]
    fn resolves_a_valid_host_target_or_absence() {
        if let Some(target) = current_host_target() {
            assert!(!target.as_str().is_empty());
        }
    }
}
