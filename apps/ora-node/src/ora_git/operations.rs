//! What each `ora-git` verb does (Node clone ADR D5–D7), run on a blocking thread.
//!
//! Git Server access runs as the Node from a per-request transport repository that borrows the
//! checkout's objects; the checkout is only touched by the workload user's hardened Git, which
//! reads the branch to push and writes `refs/remotes/ora/**`. The old value a push declares and
//! the commits a fetch negotiates from come only from the Node's ledger and the clone's base
//! commit, never from refs the Agent can write.
use super::{OraGitService, SessionGit};
use crate::git_server::{GitServerFailure, Lease, PushReport, RefUpdate, TransportRepository};
use crate::revision::DeliveryGit;
use gitlancer::{GitExecError, GitRunner};
use ora_git::{ErrorCode, Request};
use ora_node_protocol::{CommitId, WorkspaceRef};
use std::fmt::Write as _;

/// Kind of the transport repositories `ora-git` uses.
const TRANSPORT_KIND: &str = "ora-git";

/// A failed request: the code and a hint naming only refs and commits.
pub(super) type Failure = (ErrorCode, String);

/// Runs one validated request for `session`.
pub(super) fn run<R: GitRunner + Send + 'static, N: GitRunner + Send + 'static>(
    service: &OraGitService<R, N>,
    git: &(DeliveryGit<R>, N),
    session: &SessionGit,
    request: &Request,
    running: &dyn Fn() -> bool,
) -> Result<String, Failure> {
    let context = Context {
        service,
        git,
        session,
        running,
    };
    match request {
        Request::Info {} => Ok(context.info()),
        Request::List {} => context.list(),
        Request::FetchUpstream {} => context.fetch(Source::Upstream),
        Request::FetchWorkspace {
            workspace_id,
            branch,
        } => context.fetch(Source::Workspace {
            id: workspace_id,
            branch: branch.as_deref(),
        }),
        Request::Push { branch } => context.push(
            branch
                .as_deref()
                .unwrap_or_else(|| session.checkout.spec.workspace_ref.branch()),
        ),
    }
}

/// Where a fetch reads from.
enum Source<'a> {
    Upstream,
    Workspace {
        id: &'a str,
        branch: Option<&'a str>,
    },
}

/// One request's view of the service and its session.
struct Context<'a, R: Send + 'static, N: Send + 'static> {
    service: &'a OraGitService<R, N>,
    git: &'a (DeliveryGit<R>, N),
    session: &'a SessionGit,
    running: &'a dyn Fn() -> bool,
}

impl<R: GitRunner + Send + 'static, N: GitRunner + Send + 'static> Context<'_, R, N> {
    /// This Workspace's identity and where fetched refs land; no Git Server access.
    fn info(&self) -> String {
        let spec = &self.session.checkout.spec;
        let workspace = spec.workspace_ref.workspace_id();
        format!(
            "workspace {workspace}\nbranch {}\nbase {}\nupstream refs/remotes/ora/upstream/\nworkspaces refs/remotes/ora/workspaces/<workspaceId>/\n",
            spec.workspace_ref.branch(),
            spec.base_commit.as_str(),
        )
    }

    /// The upstream branches and every Workspace branch on the Git Server, with what the
    /// session's directory says about each Workspace.
    fn list(&self) -> Result<String, Failure> {
        self.require_running()?;
        let transport = self.transport(/*borrow*/ false)?;
        let binding = self.session.checkout.spec.snapshot_ref.binding_id();
        let upstream = format!("refs/ora/upstreams/{binding}/heads/");
        let refs = self
            .service
            .access
            .remote_refs(
                self.node(),
                &transport,
                &self.session.checkout.spec.repository_url,
                &self.session.execution,
                &[format!("{upstream}*"), "refs/heads/workspaces/*".to_owned()],
                &|| self.service.shutdown.requested() || !(self.running)(),
            )
            .map_err(server_failure)?;
        let mut output = String::new();
        for (name, commit) in &refs {
            if let Some(branch) = name.strip_prefix(&upstream) {
                let _ = writeln!(output, "upstream {branch} {}", commit.as_str());
            }
        }
        for (name, commit) in &refs {
            let Ok(reference) = WorkspaceRef::parse(name) else {
                continue;
            };
            let _ = write!(
                output,
                "workspace {} {} {}",
                reference.workspace_id(),
                reference.branch(),
                commit.as_str()
            );
            if let Some(listing) = self
                .session
                .directory
                .iter()
                .find(|listing| listing.workspace_id == reference.workspace_id())
            {
                let _ = write!(output, " kind={}", listing.kind);
                for (label, value) in [
                    ("issue", &listing.issue_id),
                    ("run", &listing.issue_run_id),
                    ("status", &listing.run_status),
                    ("title", &listing.issue_title),
                ] {
                    if let Some(value) = value {
                        let _ = write!(output, " {label}={value:?}");
                    }
                }
            }
            output.push('\n');
        }
        output.push_str(
            "# Issue and run details come from the Workspace directory as of when this session started.\n",
        );
        Ok(output)
    }

    /// Fetches upstream or Workspace branches into `refs/remotes/ora/...` of the checkout.
    fn fetch(&self, source: Source<'_>) -> Result<String, Failure> {
        self.require_running()?;
        let spec = &self.session.checkout.spec;
        let own = spec.workspace_ref.workspace_id();
        let (remote, local, staged, single) = match source {
            Source::Upstream => (
                format!(
                    "refs/ora/upstreams/{}/heads/",
                    spec.snapshot_ref.binding_id()
                ),
                "refs/remotes/ora/upstream/".to_owned(),
                "refs/ora/fetched/upstream/".to_owned(),
                None,
            ),
            Source::Workspace { id, branch } => (
                format!("refs/heads/workspaces/{id}/"),
                format!("refs/remotes/ora/workspaces/{id}/"),
                format!("refs/ora/fetched/workspaces/{id}/"),
                branch,
            ),
        };
        let refspec = match single {
            Some(branch) => format!("{remote}{branch}:{staged}{branch}"),
            None => format!("+{remote}*:{staged}*"),
        };
        let transport = self.transport(/*borrow*/ true)?;
        let mut known = vec![spec.base_commit.clone()];
        known.extend(
            self.service
                .ledger
                .branch_heads(&self.session.checkout_execution)
                .map_err(internal)?,
        );
        transport
            .negotiate_from(self.node(), &known.iter().collect::<Vec<_>>())
            .map_err(internal)?;
        self.service
            .access
            .fetch(
                self.node(),
                &transport,
                &spec.repository_url,
                &self.session.execution,
                &[refspec],
                &|| self.service.shutdown.requested() || !(self.running)(),
            )
            .map_err(|failure| match failure {
                GitServerFailure::MissingRef => (
                    ErrorCode::NotFound,
                    format!("no ref matches {remote}{}", single.unwrap_or("*")),
                ),
                failure => server_failure(failure),
            })?;
        let fetched = transport.refs(self.node(), &staged).map_err(internal)?;
        if fetched.is_empty() {
            return Err((
                ErrorCode::NotFound,
                format!("no ref matches {remote}{}", single.unwrap_or("*")),
            ));
        }
        transport.hand_over(self.service.owner).map_err(internal)?;
        let source = transport.path().to_str().ok_or_else(|| internal("path"))?;
        let mapping = format!("+{staged}*:{local}*");
        self.workload(&[
            "-c",
            "protocol.file.allow=always",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--",
            source,
            &mapping,
        ])
        .map_err(internal)?;
        let mut output = String::new();
        for (name, commit) in &fetched {
            let branch = &name[staged.len()..];
            // Only the session's own branches feed the ledger the next push declares.
            if remote == format!("refs/heads/workspaces/{own}/")
                && let Ok(reference) = WorkspaceRef::of(own, branch)
            {
                self.service
                    .ledger
                    .record_branch_head(&self.session.checkout_execution, &reference, commit)
                    .map_err(internal)?;
            }
            let _ = writeln!(output, "{local}{branch} {}", commit.as_str());
        }
        Ok(output)
    }

    /// Pushes a local branch to this Workspace's branch of the same name, declaring the value
    /// the ledger last saw there.
    fn push(&self, branch: &str) -> Result<String, Failure> {
        self.require_running()?;
        let spec = &self.session.checkout.spec;
        let own = spec.workspace_ref.workspace_id();
        let destination = WorkspaceRef::of(own, branch)
            .map_err(|_| (ErrorCode::Usage, "not a valid branch name".to_owned()))?;
        let peeled = format!("refs/heads/{branch}^{{commit}}");
        let commit = match self.workload(&["rev-parse", "--verify", "--quiet", &peeled]) {
            Ok(commit) => CommitId::new(commit),
            Err(GitExecError::NonZeroExit { .. }) => {
                return Err((
                    ErrorCode::NotFound,
                    format!("no local branch refs/heads/{branch}"),
                ));
            }
            Err(error) => return Err(internal(error)),
        };
        let known = self
            .service
            .ledger
            .branch_head(&self.session.checkout_execution, &destination)
            .map_err(internal)?;
        let transport = self.transport(/*borrow*/ true)?;
        // The Git Server answers every refused update the same way, so a rewrite is recognized
        // here, before pushing, against the value this checkout last saw.
        if let Some(known) = &known
            && transport
                .commit_of(self.node(), known.as_str())
                .ok()
                .flatten()
                .is_some()
            && !transport.is_ancestor(self.node(), known, &commit)
        {
            return Err((
                ErrorCode::NotFastForward,
                format!(
                    "{} is at {}, which {commit} does not contain; run `ora-git fetch workspace {own}`, merge or rebase, then push",
                    destination.as_str(),
                    known.as_str(),
                    commit = commit.as_str()
                ),
            ));
        }
        let lease = known.map_or(Lease::Absent, Lease::At);
        let pushed = self
            .service
            .access
            .push(
                self.node(),
                &transport,
                &spec.repository_url,
                &self.session.execution,
                &RefUpdate {
                    commit: &commit,
                    destination: destination.as_str(),
                    lease,
                },
                &|| self.service.shutdown.requested() || !(self.running)(),
            )
            .map_err(server_failure)?;
        match pushed {
            PushReport::Updated | PushReport::UpToDate => {}
            PushReport::Stale => {
                let current = self.remote_value(&transport, &destination)?;
                // A previous push already wrote this commit but the ledger never learned it.
                if current.as_ref() != Some(&commit) {
                    return Err((
                        ErrorCode::Conflict,
                        format!(
                            "{} is at {} on the Git Server, not where this checkout last saw it; run `ora-git fetch workspace {own}`, merge or rebase, then push",
                            destination.as_str(),
                            current.as_ref().map_or("nothing", CommitId::as_str),
                        ),
                    ));
                }
            }
            PushReport::Rejected => {
                return Err((
                    ErrorCode::Denied,
                    format!("the Git Server refused to update {}", destination.as_str()),
                ));
            }
        }
        self.service
            .ledger
            .record_branch_head(&self.session.checkout_execution, &destination, &commit)
            .map_err(internal)?;
        let tracking = format!("refs/remotes/ora/workspaces/{own}/{branch}");
        self.workload(&["update-ref", &tracking, commit.as_str()])
            .map_err(internal)?;
        Ok(format!("{} {}\n", destination.as_str(), commit.as_str()))
    }

    /// The Git Server's current value of `reference`.
    fn remote_value(
        &self,
        transport: &TransportRepository,
        reference: &WorkspaceRef,
    ) -> Result<Option<CommitId>, Failure> {
        Ok(self
            .service
            .access
            .remote_refs(
                self.node(),
                transport,
                &self.session.checkout.spec.repository_url,
                &self.session.execution,
                &[reference.as_str().to_owned()],
                &|| self.service.shutdown.requested() || !(self.running)(),
            )
            .map_err(server_failure)?
            .into_iter()
            .find(|(name, _)| name == reference.as_str())
            .map(|(_, commit)| commit))
    }

    /// The session must still be running before anything reaches the Git Server.
    fn require_running(&self) -> Result<(), Failure> {
        if (self.running)() {
            return Ok(());
        }
        Err(super::no_session())
    }

    /// A fresh transport repository for this request, borrowing the checkout's objects when the
    /// request moves objects between the checkout and the Git Server.
    fn transport(&self, borrow: bool) -> Result<TransportRepository, Failure> {
        let transport = TransportRepository::create(
            self.node(),
            &self.service.repository_root,
            TRANSPORT_KIND,
            &self.session.execution,
        )
        .map_err(internal)?;
        if borrow {
            transport
                .borrow_objects(&self.session.checkout.path.join(".git"))
                .map_err(internal)?;
        }
        Ok(transport)
    }

    /// The Node-identity runner.
    fn node(&self) -> &N {
        &self.git.1
    }

    /// Runs one hardened workload command in the checkout.
    fn workload(&self, args: &[&str]) -> Result<String, GitExecError> {
        self.git.0.output(&self.session.checkout.path, args, &[])
    }
}

/// Maps a Git Server failure to its `ora-git` code.
fn server_failure(failure: GitServerFailure) -> Failure {
    match failure {
        GitServerFailure::Denied => (
            ErrorCode::Denied,
            "the Git Server refused the request".to_owned(),
        ),
        GitServerFailure::MissingRef => (ErrorCode::NotFound, "no matching ref".to_owned()),
        GitServerFailure::Unavailable => (
            ErrorCode::Unavailable,
            "the Git Server could not be reached; try again later".to_owned(),
        ),
        GitServerFailure::Stopped => (ErrorCode::NoSession, "the Node is stopping".to_owned()),
    }
}

/// A local failure; the cause stays in the Node log.
fn internal(error: impl std::fmt::Display) -> Failure {
    ora_logging::ora_warn!(error = %error, "ora-git request failed locally");
    (ErrorCode::Internal, "the Node failed locally".to_owned())
}
