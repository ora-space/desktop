use super::super::host::SessionEnvironment;
use super::{
    PackageLocation, PackageViewLauncher, SessionPlacement, SessionWorkload,
    purge_workload_directory,
};
use ora_domain::PluginId;
use ora_node_protocol::{ExecutionId, GitIdentity};
use ora_plugin_lifecycle::{
    ChildProcessEnvironmentProvider, DenoPermission, DenoPluginRuntime, LaunchedRuntime,
    PluginLaunchRequest, PluginLogSetup, PluginRuntimeFailure, PluginRuntimeLauncher,
};
use ora_process::ProcessIdentity;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// A Node data directory with one installed package, a checkout and a workload directory.
struct Layout {
    _temp: tempfile::TempDir,
    package: PathBuf,
    checkout: PathBuf,
    workload: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let package = temp.path().join("installed").join("1.0.0");
        fs::create_dir_all(package.join("lib")).expect("package");
        fs::write(package.join("main.js"), "export {};\n").expect("entrypoint");
        fs::write(package.join("lib").join("agent.js"), "export {};\n").expect("module");
        // The Node's umask leaves installed directories owner-private.
        for directory in [package.join("lib"), package.clone()] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).expect("mode");
        }
        let checkout = temp.path().join("checkout");
        fs::create_dir_all(&checkout).expect("checkout");
        let workload = temp.path().join("agent");
        fs::create_dir_all(&workload).expect("workload directory");
        Self {
            package,
            checkout,
            workload,
            _temp: temp,
        }
    }

    /// The separate workload these tests can prepare without privilege: the test's own ids, and
    /// an inherited identity since only root may spawn as another.
    fn separate(&self) -> SessionWorkload {
        let metadata = fs::metadata(&self.checkout).expect("checkout metadata");
        SessionWorkload::Separate {
            directory: self.workload.clone(),
            identity: ProcessIdentity::Inherit,
            uid: metadata.uid(),
            gid: metadata.gid(),
        }
    }
}

/// (mode, owner uid, owner gid) of one path.
fn mode_and_owner(path: &Path) -> (u32, u32, u32) {
    let metadata = fs::symlink_metadata(path).expect("metadata");
    (metadata.mode() & 0o7777, metadata.uid(), metadata.gid())
}

/// The execution every placement in these tests belongs to.
fn execution() -> ExecutionId {
    ExecutionId::new("8b0e5a52-6f1c-4c55-9d3e-2a7b1f0c9e41")
}

/// The commit identity every environment in these tests exports.
fn git_identity() -> GitIdentity {
    GitIdentity {
        name: "Ada Lovelace".to_string(),
        email: "ada@example.com".to_string(),
    }
}

/// The session directory is a root-only-listable `0711` directory named by the execution digest,
/// holding a traversable view of the very same package files and a private home for the workload
/// user; dropping the placement removes all of it.
#[test]
fn a_separate_placement_builds_a_package_view_and_a_private_home() {
    let layout = Layout::new();
    let workload = layout.separate();
    let SessionWorkload::Separate { uid, gid, .. } = workload else {
        unreachable!("the layout prepares a separate workload");
    };

    let placement =
        SessionPlacement::prepare(&workload, &execution(), &layout.package, &layout.checkout)
            .expect("prepare");

    let root = layout
        .workload
        .join(ora_utils::hash::sha256_hex(execution().as_str().as_bytes()));
    let package = root.join("package");
    let inode = |path: PathBuf| fs::metadata(path).expect("inode").ino();
    assert_eq!(
        (
            fs::read_dir(&layout.workload)
                .expect("workload entries")
                .map(|entry| entry.expect("entry").path())
                .collect::<Vec<_>>(),
            mode_and_owner(&root).0,
            mode_and_owner(&package).0,
            mode_and_owner(&package.join("lib")).0,
            inode(package.join("main.js")),
            inode(package.join("lib").join("agent.js")),
            mode_and_owner(&root.join("home")),
        ),
        (
            vec![root.clone()],
            0o711,
            0o755,
            0o755,
            inode(layout.package.join("main.js")),
            inode(layout.package.join("lib").join("agent.js")),
            (0o700, uid, gid),
        )
    );

    drop(placement);
    assert_eq!(fs::read_dir(&layout.workload).expect("entries").count(), 0);
}

/// Whatever an earlier attempt left under the session's name is replaced, never reused.
#[test]
fn a_leftover_session_directory_is_rebuilt() {
    let layout = Layout::new();
    let root = layout
        .workload
        .join(ora_utils::hash::sha256_hex(execution().as_str().as_bytes()));
    fs::create_dir_all(root.join("home")).expect("leftover");
    fs::write(root.join("home").join("stale"), "stale").expect("stale file");

    let _placement = SessionPlacement::prepare(
        &layout.separate(),
        &execution(),
        &layout.package,
        &layout.checkout,
    )
    .expect("prepare");

    assert_eq!(
        fs::read_dir(root.join("home")).expect("home").count(),
        0,
        "the leftover home was reused"
    );
}

/// A shared placement creates nothing: the agent runs from the installed package as the Node.
#[test]
fn a_shared_placement_touches_no_directory() {
    let layout = Layout::new();

    let placement = SessionPlacement::prepare(
        &SessionWorkload::Shared,
        &execution(),
        &layout.package,
        &layout.checkout,
    )
    .expect("prepare");

    assert_eq!(
        (
            matches!(placement, SessionPlacement::Shared),
            fs::read_dir(&layout.workload).expect("entries").count()
        ),
        (true, 0)
    );
}

/// The plugin and every process the host spawns for it see the same variables: the commit
/// identity, and with a session home every per-user directory beneath it.
#[test]
fn the_session_environment_reaches_the_plugin_and_its_host_spawned_processes() {
    let home = PathBuf::from("/var/lib/ora/agent/session/home");
    let git_only = SessionEnvironment::new(&git_identity());
    let with_home = SessionEnvironment::new(&git_identity()).with_home(&home);
    let git: BTreeMap<String, String> = [
        ("GIT_AUTHOR_NAME", "Ada Lovelace"),
        ("GIT_AUTHOR_EMAIL", "ada@example.com"),
        ("GIT_COMMITTER_NAME", "Ada Lovelace"),
        ("GIT_COMMITTER_EMAIL", "ada@example.com"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect();
    let mut separate = git.clone();
    separate.extend(
        [
            ("HOME", "/var/lib/ora/agent/session/home"),
            ("XDG_CONFIG_HOME", "/var/lib/ora/agent/session/home/.config"),
            (
                "XDG_DATA_HOME",
                "/var/lib/ora/agent/session/home/.local/share",
            ),
            (
                "XDG_STATE_HOME",
                "/var/lib/ora/agent/session/home/.local/state",
            ),
            ("XDG_CACHE_HOME", "/var/lib/ora/agent/session/home/.cache"),
            ("DENO_DIR", "/var/lib/ora/agent/session/home/.cache/deno"),
            ("DENO_NO_UPDATE_CHECK", "1"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string())),
    );
    let both = |environment: &SessionEnvironment| {
        (
            environment.plugin_environment("official/ora-space.echo"),
            environment
                .environment("official/ora-space.echo", Path::new("/checkout"))
                .expect("environment"),
        )
    };

    assert_eq!(
        (both(&git_only), both(&with_home)),
        ((git.clone(), git), (separate.clone(), separate))
    );
}

/// Records the request it is asked to launch and fails, standing in for Deno.
#[derive(Clone, Default)]
struct CapturingLauncher(Arc<Mutex<Vec<PluginLaunchRequest>>>);

impl PluginRuntimeLauncher for CapturingLauncher {
    type Runtime = DenoPluginRuntime;

    fn launch(
        &self,
        request: PluginLaunchRequest,
        _log: PluginLogSetup,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        async { Err(PluginRuntimeFailure::new("captured")) }
    }
}

/// A launch request for the plugin installed at `package_root`.
fn request(package_root: &Path) -> PluginLaunchRequest {
    PluginLaunchRequest {
        plugin_id: PluginId::parse("official/ora-space.echo").expect("plugin id"),
        deno_path: PathBuf::from("/opt/ora/bin/deno"),
        entrypoint: package_root.join("main.js"),
        package_root: package_root.to_path_buf(),
        permissions: vec![DenoPermission::AllowRun, DenoPermission::AllowEnv],
        data_dir: PathBuf::from("/var/lib/ora/node/plugins/data/official/ora-space.echo"),
        allow_childprocess: true,
    }
}

/// Launches `request` through a view launcher and returns the failure and what reached Deno.
async fn launch_through(
    location: PackageLocation,
    request: PluginLaunchRequest,
) -> (PluginRuntimeFailure, Vec<PluginLaunchRequest>) {
    let inner = CapturingLauncher::default();
    let launcher = PackageViewLauncher {
        inner: inner.clone(),
        location,
    };
    let log_root = PathBuf::from("/var/lib/ora/node/logs");
    let failure = match launcher
        .launch(
            request,
            PluginLogSetup {
                directory: log_root.join("official"),
                root: log_root,
                host_session_id: "session".to_string(),
                generation: 1,
                level: tokio::sync::watch::channel(ora_logging::LogLevel::Info).1,
            },
        )
        .await
    {
        Ok(_) => panic!("the capturing launcher always fails"),
        Err(failure) => failure,
    };
    let captured = inner
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (failure, captured)
}

/// The launch moves into the view — working directory, entrypoint and `packageCommand` root —
/// and carries everything else unchanged; any other package is refused before Deno is reached.
#[tokio::test]
async fn the_view_launcher_moves_the_launch_into_the_package_view() {
    let layout = Layout::new();
    let view = layout.workload.join("session").join("package");
    let location = || PackageLocation::View {
        installed: layout.package.clone(),
        view: view.clone(),
    };
    let other = layout.checkout.clone();

    let moved = launch_through(location(), request(&layout.package)).await;
    let refused = launch_through(location(), request(&other)).await;
    let installed = launch_through(PackageLocation::Installed, request(&layout.package)).await;

    assert_eq!(
        (moved, refused, installed),
        (
            (
                PluginRuntimeFailure::new("captured"),
                vec![PluginLaunchRequest {
                    entrypoint: view.join("main.js"),
                    package_root: view.clone(),
                    ..request(&layout.package)
                }],
            ),
            (
                PluginRuntimeFailure::new("the launched package is not the session's package view"),
                Vec::new(),
            ),
            (
                PluginRuntimeFailure::new("captured"),
                vec![request(&layout.package)],
            ),
        )
    );
}

/// Opening the service removes every session directory a previous process left, links included
/// and never followed.
#[test]
fn purging_removes_every_leftover_without_following_links() {
    let layout = Layout::new();
    let outside = layout.checkout.join("keep");
    fs::write(&outside, "keep").expect("outside file");
    fs::create_dir_all(layout.workload.join("old").join("home")).expect("old session");
    fs::write(layout.workload.join("old").join("home").join("cache"), "x").expect("cache");
    symlink(&layout.checkout, layout.workload.join("link")).expect("link");
    fs::write(layout.workload.join("stray"), "stray").expect("stray file");

    let removed = purge_workload_directory(&layout.workload).expect("purge");

    assert_eq!(
        (
            removed,
            fs::read_dir(&layout.workload).expect("entries").count(),
            fs::read_to_string(&outside).expect("outside survives"),
        ),
        (3, 0, "keep".to_string())
    );
}
