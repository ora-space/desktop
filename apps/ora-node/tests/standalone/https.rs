//! The Git Server a standalone Node clones from: the shared smart-HTTP double with a published
//! Binding snapshot, plus a gate that holds the workload's checkout fetch for interruption tests.
use ora_node::GitAccessConfig;
use ora_node_protocol::*;
use ora_test_support::git_server::{GitServer, git};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The Binding every fixture snapshot belongs to.
pub const BINDING: &str = "9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a";
/// The Workspace every fixture clone is for.
pub const WORKSPACE: &str = "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a";
/// The Project repository on the double.
const REPOSITORY: &str = "tenant/project.git";

/// A running Git Server double holding one Project repository.
pub struct GitServerFixture {
    pub server: GitServer,
    /// The repository directory on the double, for direct inspection and setup.
    pub repository: PathBuf,
    pub url: GitRepositoryUrl,
}

impl GitServerFixture {
    /// Starts the double under `root` and publishes `source`'s HEAD as the Binding snapshot.
    pub fn new(root: &Path, source: PathBuf) -> Self {
        let directory = root.join("git-server");
        fs::create_dir_all(&directory).unwrap();
        let server = GitServer::start(&directory);
        let repository = server.create_repository(REPOSITORY);
        let url = GitRepositoryUrl::parse(&server.url(REPOSITORY)).unwrap();
        let fixture = Self {
            server,
            repository,
            url,
        };
        fixture.publish(&source, "HEAD");
        fixture
    }

    /// Writes `revision` of `source` as a snapshot ref, as the external line would, without the
    /// Git Server's sandbox update rules; returns the snapshot commit.
    pub fn publish(&self, source: &Path, revision: &str) -> CommitId {
        let commit = git(source, &["rev-parse", &format!("{revision}^{{commit}}")]);
        let source = source.to_str().unwrap();
        git(
            &self.repository,
            &[
                "fetch",
                "--quiet",
                source,
                &format!("+{commit}:{}", snapshot_ref(&commit)),
            ],
        );
        CommitId::new(commit)
    }

    /// The git-access identity the Node presents to this double.
    pub fn access(&self) -> GitAccessConfig {
        let material = self.server.access();
        GitAccessConfig {
            ca_cert: material.ca_cert.clone(),
            client_cert: material.client_cert.clone(),
            client_key: material.client_key.clone(),
        }
    }

    /// The clone input of the published snapshot `commit` for `branch` of the fixture Workspace.
    pub fn spec(&self, node: &str, commit: &CommitId, branch: &str) -> CloneExecutionSpec {
        CloneExecutionSpec {
            node_id: NodeId::new(node),
            repository_url: self.url.clone(),
            snapshot_ref: SnapshotRef::parse(&snapshot_ref(commit.as_str())).unwrap(),
            base_commit: commit.clone(),
            workspace_ref: WorkspaceRef::of(WORKSPACE, branch).unwrap(),
        }
    }

    /// The commit the snapshot of the repository's current HEAD names.
    pub fn snapshot(&self) -> CommitId {
        let refs = git(
            &self.repository,
            &[
                "for-each-ref",
                "--format=%(objectname)",
                "refs/ora/upstreams/",
            ],
        );
        CommitId::new(refs.lines().last().unwrap())
    }

    /// Answers every request with 403 while set, as the Git Server does for a refused execution.
    pub fn reject_auth(&self, reject: bool) {
        self.server.answer_always(reject.then_some(403));
    }

    /// Stops accepting connections while set.
    pub fn pause(&self, paused: bool) {
        self.server.pause(paused);
    }

    /// The flag behind [`GitServerFixture::pause`].
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.server.pause_flag()
    }
}

/// The snapshot ref of `commit` in the fixture Binding.
fn snapshot_ref(commit: &str) -> String {
    format!("refs/ora/upstreams/{BINDING}/snapshots/{commit}")
}

/// Holds the directly managed workload Git before its checkout fetch until released.
///
/// Pausing an upload-pack child makes termination racy: Git can observe the child's death and
/// exit with its own error before the host signals Git. These tests require direct-process signal
/// evidence, so the fixture gates the managed program itself, preserving the real Git command.
pub struct WorkloadGate {
    pub ready: PathBuf,
    release: PathBuf,
}

impl WorkloadGate {
    /// Wraps only the workload's file fetch; transport HTTPS commands still run real Git immediately.
    pub fn install(root: &Path) -> Self {
        let gate = Self {
            ready: root.join("gate-ready"),
            release: root.join("gate-release"),
        };
        let program = root.join("git");
        // Git dispatches `git-<name>` basenames as subcommands; retain the executable's name.
        let real_git = root.join("unwrapped").join("git");
        fs::create_dir(real_git.parent().unwrap()).unwrap();
        fs::rename(&program, &real_git).unwrap();
        fs::write(
            &program,
            format!(
                "#!/bin/sh\npause=false\nfor arg do\n  if test \"$arg\" = protocol.file.allow=always; then pause=true; fi\ndone\nif test \"$pause\" = true; then\n  printf ready > '{}'\n  while test ! -f '{}'; do /bin/sleep 0.02; done\nfi\nexec '{}' \"$@\"\n",
                gate.ready.display(),
                gate.release.display(),
                real_git.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        gate
    }
}
