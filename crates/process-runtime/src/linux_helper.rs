use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use ora_utils::path::{TrustedPathKind, open_trusted_path};
use ora_utils::process::LinuxChildIdentity;
use serde::Deserialize;

mod launch;
mod management;
mod service;
pub use launch::spawn_linux_helper_workload;
pub use service::serve_linux_helper;

/// Administrator-owned deployment policy, never accepted from a workload launch request.
pub struct LinuxHelperConfig(RawConfig);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    version: u32,
    manager_uid: u32,
    workload_uid: u32,
    workload_gid: u32,
    cgroup_root: PathBuf,
}

impl LinuxHelperConfig {
    /// Validates deployment identity separation before any privileged filesystem operations.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let config: RawConfig = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if config.version != 1 {
            return Err("unsupported helper deployment version".into());
        }
        LinuxChildIdentity::new(config.workload_uid, config.workload_gid)
            .map_err(|error| error.to_string())?;
        if config.manager_uid == 0
            || config.manager_uid == u32::MAX
            || config.manager_uid == config.workload_uid
        {
            return Err("helper requires distinct non-root manager and workload identities".into());
        }
        Ok(Self(config))
    }

    /// Checks deployment prerequisites without creating, moving or killing any process.
    ///
    /// This is a point-in-time diagnostic, not authorization to launch or evidence of quiescence.
    /// The eventual launcher must retain descriptors and revalidate at its side-effect boundary.
    pub fn verify_deployment(&self) -> Result<(), String> {
        let root = open_trusted_path(
            &self.0.cgroup_root,
            /*owner*/ 0,
            TrustedPathKind::Directory,
        )
        .map_err(|error| error.to_string())?;
        let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: root is a live descriptor and fstatfs initializes the supplied output on success.
        if unsafe { libc::fstatfs(root.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: fstatfs succeeded, so its output is initialized.
        if unsafe { filesystem.assume_init() }.f_type != libc::CGROUP2_SUPER_MAGIC {
            return Err("workload root is not a cgroup v2 filesystem".into());
        }
        for (name, expected) in [("cgroup.type", "domain"), ("cgroup.procs", "")] {
            let file = open_trusted_path(
                &self.0.cgroup_root.join(name),
                /*owner*/ 0,
                TrustedPathKind::File,
            )
            .map_err(|error| error.to_string())?;
            let mut value = String::new();
            file.take(/*limit*/ 4097)
                .read_to_string(&mut value)
                .map_err(|error| error.to_string())?;
            if value.len() > 4096 || value.trim() != expected {
                return Err(format!("workload root has incompatible {name}"));
            }
        }
        // cgroup.events and cgroup.kill are absent at the hierarchy root; requiring both keeps
        // deployment scoped to a dedicated non-root subtree. No control file is written here.
        for (name, kind) in [
            ("cgroup.events", TrustedPathKind::File),
            ("cgroup.kill", TrustedPathKind::WritableFile),
        ] {
            open_trusted_path(&self.0.cgroup_root.join(name), /*owner*/ 0, kind)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

/// Validates administrator-owned configuration for an explicitly invoked root helper deployment.
///
/// Refuses setuid invocation; installation must use a separately managed root process. Success
/// does not advertise Strong: workload launch and guardian integration are separate obligations.
pub fn check_linux_helper_deployment(config_path: &Path) -> Result<(), String> {
    load_deployment(config_path)?.verify_deployment()
}

/// Keeps diagnostic and serving entry points behind the same root-owned configuration gate.
fn load_deployment(config_path: &Path) -> Result<LinuxHelperConfig, String> {
    // SAFETY: these identity queries take no pointers and have no side effects.
    if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
        return Err("helper deployment check requires a separately managed root process".into());
    }
    let file = open_trusted_path(config_path, /*owner*/ 0, TrustedPathKind::File)
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(/*limit*/ 16385)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 16384 {
        return Err("helper deployment configuration exceeds 16 KiB".into());
    }
    LinuxHelperConfig::from_json(&bytes)
}
