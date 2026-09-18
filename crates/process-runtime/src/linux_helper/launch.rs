use std::ffi::CString;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path};
use std::process::{Child, Command, Stdio};

use ora_process_protocol::RunSpec;
use ora_utils::path::{TrustedPathKind, open_trusted_path};
use ora_utils::process::LinuxChildIdentity;

use super::load_deployment;

/// Low-level launch gate for a previously provisioned `root/scope/run` cgroup, not a Run API.
///
/// Only trusted helper code may select the target. The caller must exclusively own its launch
/// admission, keep durable responsibility, drain all three pipes, reap the direct child and retain
/// the cgroup until descendants are gone. An error does not prove the cgroup is safe to retire.
/// This is deliberately not exposed by the management protocol and does not advertise Strong.
/// As a raw pipe handoff, it does not enforce `RunSpec.output`; that policy belongs to its caller.
pub fn spawn_linux_helper_workload(
    config_path: &Path,
    run_cgroup: &Path,
    spec: &RunSpec,
) -> Result<Child, String> {
    let config = load_deployment(config_path)?;
    config.verify_deployment()?;
    let relative = run_cgroup
        .strip_prefix(&config.0.cgroup_root)
        .map_err(|_| "run is outside helper root")?;
    let components: Vec<_> = relative.components().collect();
    if components.len() != 2
        || !components
            .iter()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("run must be directly inside a scope below the helper root".into());
    }
    // Absolute commands avoid a privileged helper's PATH becoming executable selection policy.
    // chdir happens after dropping credentials, not in Command's earlier root-owned setup.
    if !Path::new(&spec.program).is_absolute() || !spec.cwd.is_absolute() {
        return Err("helper launch requires an absolute executable and working directory".into());
    }
    let cwd = CString::new(spec.cwd.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    let identity = LinuxChildIdentity::new(config.0.workload_uid, config.0.workload_gid)
        .map_err(|error| error.to_string())?;
    let scope = run_cgroup.parent().ok_or("run needs a scope")?;
    // Protect every common migration ancestor inside the deployment, not just the leaf file.
    for directory in [config.0.cgroup_root.as_path(), scope, run_cgroup] {
        for (name, expected) in [
            ("cgroup.type", "domain"),
            ("cgroup.procs", ""),
            ("cgroup.freeze", "0"),
        ] {
            let file = open_trusted_path(
                &directory.join(name),
                /*owner*/ 0,
                TrustedPathKind::File,
            )
            .map_err(|error| error.to_string())?;
            let mut value = String::new();
            file.take(/*limit*/ 4097)
                .read_to_string(&mut value)
                .map_err(|error| error.to_string())?;
            if value.len() > 4096 || value.trim() != expected {
                return Err(format!("launch boundary has incompatible {name}"));
            }
        }
    }
    let events = open_trusted_path(
        &run_cgroup.join("cgroup.events"),
        /*owner*/ 0,
        TrustedPathKind::File,
    )
    .map_err(|error| error.to_string())?;
    let mut value = String::new();
    events
        .take(/*limit*/ 4097)
        .read_to_string(&mut value)
        .map_err(|error| error.to_string())?;
    if value.len() > 4096
        || !value.lines().any(|line| line == "populated 0")
        || !value.lines().any(|line| line == "frozen 0")
    {
        return Err("run cgroup is not proven empty and unfrozen before launch".into());
    }
    let _kill = open_trusted_path(
        &run_cgroup.join("cgroup.kill"),
        /*owner*/ 0,
        TrustedPathKind::WritableFile,
    )
    .map_err(|error| error.to_string())?;
    let procs = open_trusted_path(
        &run_cgroup.join("cgroup.procs"),
        /*owner*/ 0,
        TrustedPathKind::WritableFile,
    )
    .map_err(|error| error.to_string())?;
    let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: procs remains live and the kernel initializes the output on success.
    if unsafe { libc::fstatfs(procs.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    // SAFETY: fstatfs succeeded; a nested ordinary mount cannot masquerade as the launch gate.
    if unsafe { filesystem.assume_init() }.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err("run membership file is not on cgroup v2".into());
    }
    // The helper may have closed stdio. Keep the membership descriptor above 2 so Command's
    // pipe setup cannot replace it and turn the membership write into an ordinary stdout write.
    // SAFETY: fcntl duplicates the live descriptor without modifying the original.
    let descriptor = unsafe { libc::fcntl(procs.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    // SAFETY: fcntl returned a fresh owned descriptor, transferred exactly once.
    let membership = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .env_clear()
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: captures were allocated before fork. The closure only makes async-signal-safe
    // syscalls with live descriptors/buffers, and every failure aborts exec through Command.
    unsafe {
        command.pre_exec(move || {
            // The kernel interprets 0 as the writing process itself, avoiding PID lookup/reuse.
            // No untrusted code has run: membership is established before the privilege drop.
            let written = libc::write(
                membership.as_raw_fd(),
                b"0".as_ptr().cast(),
                /*count*/ 1,
            );
            if written < 0 {
                return Err(io::Error::last_os_error());
            }
            if written != 1 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            identity.enter_child()?;
            if libc::chdir(cwd.as_ptr()) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().map_err(|error| error.to_string())
}
