//! The Node side of `ora-git` (Node clone ADR D5): one Unix socket per running Agent session.
//!
//! A session's socket lives under the deployment's `ora-git` directory with a random name, and
//! its path reaches the agent plugin as `ORA_GIT_SOCKET`. A request belongs to the session whose
//! socket it arrived on, never to anything it declares; the peer must be the workload user, and
//! the session must still be running. One request runs at a time per session. The socket is
//! removed when the session leaves the runtime, and the directory is cleared when the Node starts.

mod operations;

use crate::Shutdown;
use crate::git_server::GitServerAccess;
use crate::revision::DeliveryGit;
use gitlancer::GitRunner;
use ora_git::{ErrorCode, MAX_REQUEST_BYTES, Reply, Request};
use ora_node_db::{ClonedCheckout, SessionJournal};
use ora_node_protocol::{ExecutionId, WorkspaceListing};
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// What a session's requests act on, all from the Node's own records.
#[derive(Clone, Debug)]
pub(crate) struct SessionGit {
    /// The session execution, whose ID the Git Server authorizes requests by.
    pub(crate) execution: ExecutionId,
    /// The clone execution whose checkout the session runs in.
    pub(crate) checkout_execution: ExecutionId,
    pub(crate) checkout: ClonedCheckout,
    /// Shown by `ora-git list`, never used for a decision.
    pub(crate) directory: Vec<WorkspaceListing>,
}

/// Opens the per-session sockets a session driver hands to its agent.
///
/// The session layer depends on this trait rather than the service's Git runners, so the session
/// stays generic over its own collaborators only. Implementations create a socket only the
/// expected workload user can use and remove it when the returned handle is dropped.
pub(crate) trait OraGitBroker: Send + Sync + 'static {
    /// Starts serving `session` on a new socket; must be called inside a Tokio runtime.
    fn open(&self, session: SessionGit) -> io::Result<OraGitSocket>;
}

/// A session's socket; dropping it stops serving and removes the file.
pub(crate) struct OraGitSocket {
    path: PathBuf,
    running: Arc<AtomicBool>,
    /// The session's one request slot, held by the request's Git until it ends.
    slot: Arc<Semaphore>,
    task: tokio::task::JoinHandle<()>,
}

impl OraGitSocket {
    /// The path exported as `ORA_GIT_SOCKET`.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Ends the session's access: new requests fail as `no_session` at once, and a request whose
    /// Git is already running is waited for, so nothing the session asked for still writes its
    /// checkout or the Git Server once the session is recorded as ended and delivery may start.
    /// That wait is bounded by the request's Git Server time limit.
    pub(crate) async fn close(self) {
        self.running.store(false, Ordering::SeqCst);
        let slot = Arc::clone(&self.slot);
        let _idle = slot.acquire().await;
        drop(self);
    }
}

impl Drop for OraGitSocket {
    /// Ends the session's access first, so a request already running sees it, then removes the
    /// socket so new connections fail.
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.task.abort();
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            ora_logging::ora_warn!(error = %error, "ora-git socket was not removed");
        }
    }
}

/// Everything `ora-git` requests run with, shared by every session of the Node.
pub(crate) struct OraGitService<R: Send + 'static, N: Send + 'static> {
    directory: PathBuf,
    /// The only UID a request may come from.
    peer_uid: u32,
    /// One request's Git at a time across sessions: host-backed runners drive their own runtime.
    git: Mutex<Option<(DeliveryGit<R>, N)>>,
    access: GitServerAccess,
    repository_root: PathBuf,
    /// The workload user transport repositories are handed to before the checkout reads them.
    owner: Option<u32>,
    ledger: SessionJournal,
    shutdown: Shutdown,
}

/// The configuration [`OraGitService::new`] composes from.
pub(crate) struct OraGitParts<R: Send + 'static, N: Send + 'static> {
    pub(crate) directory: PathBuf,
    pub(crate) peer_uid: u32,
    pub(crate) workload: DeliveryGit<R>,
    pub(crate) node: N,
    pub(crate) access: GitServerAccess,
    pub(crate) repository_root: PathBuf,
    pub(crate) owner: Option<u32>,
    pub(crate) ledger: SessionJournal,
    pub(crate) shutdown: Shutdown,
}

impl<R: Send + 'static, N: Send + 'static> OraGitService<R, N> {
    /// Prepares the socket directory: Node-owned, traversable but not listable by others, and
    /// emptied of every socket a previous Node process left.
    pub(crate) fn new(parts: OraGitParts<R, N>) -> io::Result<Self> {
        match fs::remove_dir_all(&parts.directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            // A deployment-mounted directory itself may not be removable; its contents are.
            Err(_) => {
                for entry in fs::read_dir(&parts.directory)? {
                    let _ = fs::remove_file(entry?.path());
                }
            }
        }
        match fs::DirBuilder::new()
            .recursive(/*recursive*/ true)
            .mode(/*mode*/ 0o711)
            .create(&parts.directory)
        {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        fs::set_permissions(&parts.directory, fs::Permissions::from_mode(/*mode*/ 0o711))?;
        Ok(Self {
            directory: parts.directory,
            peer_uid: parts.peer_uid,
            git: Mutex::new(Some((parts.workload, parts.node))),
            access: parts.access,
            repository_root: parts.repository_root,
            owner: parts.owner,
            ledger: parts.ledger,
            shutdown: parts.shutdown,
        })
    }
}

impl<R, N> OraGitBroker for Arc<OraGitService<R, N>>
where
    R: GitRunner + Send + 'static,
    N: GitRunner + Send + 'static,
{
    /// Binds a fresh socket the workload user can connect to and serves it until dropped.
    fn open(&self, session: SessionGit) -> io::Result<OraGitSocket> {
        let path = self
            .directory
            .join(format!("{}.sock", uuid::Uuid::new_v4().simple()));
        let listener = UnixListener::bind(&path)?;
        // Connecting needs write access to the socket; who may use it is decided by the peer
        // check, which no file mode can replace.
        fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o666))?;
        let running = Arc::new(AtomicBool::new(true));
        let slot = Arc::new(Semaphore::new(/*permits*/ 1));
        let session = Arc::new(Serving {
            service: Arc::clone(self),
            session,
            running: Arc::clone(&running),
            slot: Arc::clone(&slot),
        });
        let task = tokio::spawn(async move {
            // The listener owns its connections. Aborting it on session exit also closes peers
            // that have connected without finishing a request, rather than retaining the broker.
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        let session = Arc::clone(&session);
                        connections.spawn(async move {
                            let _ = session.serve(stream).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(OraGitSocket {
            path,
            running,
            slot,
            task,
        })
    }
}

impl<R: Send + 'static, N: Send + 'static> Drop for OraGitService<R, N> {
    /// Managed Git runners own synchronous runtimes. The last session handle may be released on
    /// an async worker, so retire those runtimes on a joined thread that permits blocking cleanup.
    fn drop(&mut self) {
        if let Some(git) = self
            .git
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = std::thread::spawn(move || drop(git)).join();
        }
    }
}

/// One session's socket state.
struct Serving<R: Send + 'static, N: Send + 'static> {
    service: Arc<OraGitService<R, N>>,
    session: SessionGit,
    running: Arc<AtomicBool>,
    slot: Arc<Semaphore>,
}

impl<R, N> Serving<R, N>
where
    R: GitRunner + Send + 'static,
    N: GitRunner + Send + 'static,
{
    /// Answers one connection with exactly one result line.
    async fn serve(self: Arc<Self>, mut stream: UnixStream) -> io::Result<()> {
        let mut slot = None;
        let reply = Arc::clone(&self).answer(&mut stream, &mut slot).await;
        let reply = match reply {
            Ok(output) => Reply::Done { output },
            Err((code, message)) => Reply::Failed { code, message },
        };
        let mut line = serde_json::to_vec(&reply).map_err(io::Error::other)?;
        line.push(b'\n');
        let written = stream.write_all(&line).await;
        // The request's slot is given back only once its answer is out, so a closing socket
        // never cuts off the answer of a request it waited for.
        drop(slot);
        written?;
        stream.shutdown().await
    }

    /// Checks the peer, the session and the request, then runs it off the async workers; the
    /// session's slot the request ran in ends up in `held`.
    async fn answer(
        self: Arc<Self>,
        stream: &mut UnixStream,
        held: &mut Option<OwnedSemaphorePermit>,
    ) -> Result<String, operations::Failure> {
        let peer = stream.peer_cred().map_err(|_| {
            (
                ErrorCode::Internal,
                "peer credentials unavailable".to_owned(),
            )
        })?;
        if peer.uid() != self.service.peer_uid {
            return Err((
                ErrorCode::Forbidden,
                "ora-git serves only the workload user".to_owned(),
            ));
        }
        if !self.running.load(Ordering::SeqCst) {
            return Err(no_session());
        }
        let mut line = String::new();
        let mut reader = BufReader::new(stream).take(MAX_REQUEST_BYTES as u64);
        reader
            .read_line(&mut line)
            .await
            .map_err(|_| (ErrorCode::Usage, "the request could not be read".to_owned()))?;
        if !line.ends_with('\n') {
            return Err((
                ErrorCode::Usage,
                "the request is not one line within 4 KiB".to_owned(),
            ));
        }
        let request: Request = serde_json::from_str(line.trim_end())
            .map_err(|_| (ErrorCode::Usage, ora_git::USAGE.to_owned()))?;
        request
            .validate()
            .map_err(|message| (ErrorCode::Usage, message))?;
        // A closing socket holds the slot too; its waiting peers learn the session ended.
        let Ok(slot) = Arc::clone(&self.slot).try_acquire_owned() else {
            if !self.running.load(Ordering::SeqCst) {
                return Err(no_session());
            }
            return Err((
                ErrorCode::Busy,
                "another ora-git request of this session is running".to_owned(),
            ));
        };
        let serving = Arc::clone(&self);
        // The slot moves into the blocking work and comes back with its result: it is released
        // no earlier than the Git ends, even if this connection is dropped first, which is what
        // closing the socket waits for.
        let finished = tokio::task::spawn_blocking(move || {
            // Running means the socket is open, the Node is not stopping, and the ledger has
            // neither an end nor a requested end for the session.
            let running = || {
                serving.running.load(Ordering::SeqCst)
                    && !serving.service.shutdown.requested()
                    && serving
                        .service
                        .ledger
                        .session_running(&serving.session.execution)
                        .unwrap_or(false)
            };
            // The lock only serializes Git; a poisoned one still holds usable runners.
            let git = serving
                .service
                .git
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let result = match git.as_ref() {
                Some(git) => {
                    operations::run(&serving.service, git, &serving.session, &request, &running)
                }
                None => Err((ErrorCode::Internal, "Git service closed".to_owned())),
            };
            (result, slot)
        })
        .await;
        match finished {
            Ok((result, slot)) => {
                *held = Some(slot);
                result
            }
            Err(_) => Err((ErrorCode::Internal, "the request stopped".to_owned())),
        }
    }
}

/// The answer for a session that no longer runs.
fn no_session() -> operations::Failure {
    (
        ErrorCode::NoSession,
        "the session is no longer running".to_owned(),
    )
}

#[cfg(test)]
mod tests;
