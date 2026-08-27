//! Resolves the canonical Rust target triple of the host the registry is running on.

use ora_plugin_manifest::HookTarget;

/// Returns the canonical Rust target triple for the host the backend is running on.
///
/// The triple is a compile-time constant of the host binary, so it is always available and never
/// depends on runtime environment inspection. Target selection uses this exact triple; it never
/// falls back across architecture, operating system, libc, or ABI.
pub fn current_host_target() -> HookTarget {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    const HOST_TRIPLE: &str = "x86_64-pc-windows-msvc";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    const HOST_TRIPLE: &str = "aarch64-apple-darwin";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    const HOST_TRIPLE: &str = "x86_64-apple-darwin";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const HOST_TRIPLE: &str = "x86_64-unknown-linux-gnu";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    const HOST_TRIPLE: &str = "aarch64-unknown-linux-gnu";
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
    )))]
    const HOST_TRIPLE: &str = "unsupported-host";

    HookTarget::parse(HOST_TRIPLE)
        .unwrap_or_else(|error| unreachable!("host triple is valid: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::current_host_target;

    /// The host target is always available and valid.
    #[test]
    fn resolves_a_valid_host_target() {
        let target = current_host_target();
        assert!(!target.as_str().is_empty());
    }
}
