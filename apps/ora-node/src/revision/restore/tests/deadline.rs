//! Cross-step deadline tests retain real repositories while scripting execution timing.
use super::*;
use crate::revision::restore::{ObjectDownloader, RestoreGitRunner};
use gitlancer::{GitCommand, GitExecError, GitOutput, GitRunner};
use ora_utils::http::{FetchError, FetchOutcome};
use pretty_assertions::assert_eq;
use std::time::Instant;
use tracing::instrument::WithSubscriber;

/// A delayed successful download makes its time consumption visible to the first Git command.
struct DelayedDownload {
    inner: HttpDownloader,
    delay: Duration,
}
impl DelayedDownload {
    /// Delays the real HTTP fixture transfer, including its size and digest verification.
    fn new(delay: Duration) -> Self {
        Self {
            inner: HttpDownloader::new(
                ReqwestFetcher::new(ProxyConfig::default()),
                FetchOptions {
                    connect_timeout: Duration::from_secs(/*secs*/ 5),
                    total_timeout: Duration::from_secs(/*secs*/ 10),
                },
            ),
            delay,
        }
    }
}
impl ObjectDownloader for DelayedDownload {
    /// Streams are tested separately; this double supplies verified bytes after a controlled wait.
    async fn get(
        &self,
        grant: &ObjectDownloadGrant,
        destination: &Path,
        limit: u64,
    ) -> Result<FetchOutcome, FetchError> {
        tokio::time::sleep(self.delay).await;
        self.inner.get(grant, destination, limit).await
    }
}

/// Records actual command admission times and the cutoff passed to the execution owner.
struct TimedGit {
    calls: Arc<Mutex<Vec<(Instant, Instant)>>>,
    delay: Duration,
}
impl GitRunner for TimedGit {
    /// The fixture keeps real Git semantics; its restore entry point supplies timing.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        CliGitRunner.run(command)
    }
}
impl RestoreGitRunner for TimedGit {
    /// Models execution consuming time, with a typed timeout before a late result can escape.
    fn run_bounded_before(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        deadline: Instant,
    ) -> Result<GitOutput, GitExecError> {
        self.calls.lock().unwrap().push((Instant::now(), deadline));
        std::thread::sleep(self.delay);
        super::super::super::runner::check(deadline)?;
        self.run_bounded(command, stdout, stderr)
    }
}

/// Models execution finishing within budget followed by a slower original Scope close barrier.
struct LateCleanup;
impl GitRunner for LateCleanup {
    /// Real Git supplies the execution result before the simulated cleanup wait.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        CliGitRunner.run(command)
    }
}
impl RestoreGitRunner for LateCleanup {
    /// Cleanup is deliberately outside the execution clock; it cannot manufacture a timeout.
    fn run_bounded_before(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        deadline: Instant,
    ) -> Result<GitOutput, GitExecError> {
        let output = self.run_bounded(command, stdout, stderr)?;
        super::super::super::runner::check(deadline)?;
        if command.args.iter().any(|arg| arg == "merge-base") {
            std::thread::sleep(Duration::from_millis(/*millis*/ 5200));
        }
        Ok(output)
    }
}

/// A successful final command is allowed to finish its independent cleanup after the cutoff.
#[tokio::test]
async fn final_command_cleanup_does_not_consume_the_execution_budget() {
    async {
        let world = World::new();
        let checkout = world.clone_into("cleanup-budget");
        let store = ObjectStore::start(fs::read(world.bundle()).unwrap(), &[]).await;
        let grants = DownloadGrants::new();
        controller(
            &grants,
            &store,
            world.prior.bundle.key.clone(),
            /*refuse*/ false,
        );
        let restore = RevisionRestorer::new(
            DeliveryGit::new(
                LateCleanup,
                GitPolicy {
                    env: GitEnv::automation_defaults()
                        .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                        .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
                    config_args: vec![],
                },
            ),
            DelayedDownload::new(Duration::ZERO),
            grants,
            NodeId::new("node"),
            world.path("cleanup-home").join(RESTORE_ROOT),
            /*owner*/ None,
            RestorePolicy {
                deadline: Duration::from_secs(/*secs*/ 5),
                ..POLICY
            },
        );
        let started = Instant::now();
        assert_eq!(
            restore.restore(request(&world, checkout)).await,
            Ok(Restored::OnRemoteHistory)
        );
        assert!(started.elapsed() >= Duration::from_millis(/*millis*/ 5200));
    }
    .with_subscriber(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    )
    .await;
}

/// Compose a restore over the same hardened fixture policy used by repository tests.
fn subject<D>(
    world: &World,
    grants: &DownloadGrants,
    calls: Arc<Mutex<Vec<(Instant, Instant)>>>,
    downloader: D,
    delay: Duration,
    policy: RestorePolicy,
) -> RevisionRestorer<TimedGit, D> {
    RevisionRestorer::new(
        DeliveryGit::new(
            TimedGit { calls, delay },
            GitPolicy {
                env: GitEnv::automation_defaults()
                    .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                    .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
                config_args: vec![],
            },
        ),
        downloader,
        grants.clone(),
        NodeId::new("node"),
        world.path("deadline-home").join(RESTORE_ROOT),
        /*owner*/ None,
        policy,
    )
}

/// Download and successive Git steps consume one clock; none can renew it on admission.
#[tokio::test]
async fn download_and_git_steps_share_one_absolute_cutoff() {
    async {
        let world = World::new();
        let checkout = world.clone_into("shared-cutoff");
        let store = ObjectStore::start(fs::read(world.bundle()).unwrap(), &[]).await;
        let grants = DownloadGrants::new();
        controller(
            &grants,
            &store,
            world.prior.bundle.key.clone(),
            /*refuse*/ false,
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let policy = RestorePolicy {
            deadline: Duration::from_secs(/*secs*/ 5),
            ..POLICY
        };
        let restore = subject(
            &world,
            &grants,
            Arc::clone(&calls),
            DelayedDownload::new(Duration::from_secs(/*secs*/ 1)),
            Duration::from_secs(/*secs*/ 1),
            policy,
        );
        assert_eq!(
            restore.restore(request(&world, checkout)).await,
            Err(RestoreFailure::Unavailable)
        );
        let calls = calls.lock().unwrap();
        assert!(
            !calls.is_empty() && calls.len() <= 4,
            "unexpected admitted commands: {calls:?}"
        );
        let cutoff = calls[0].1;
        assert!(cutoff.duration_since(calls[0].0) < Duration::from_millis(/*millis*/ 4500));
        assert!(
            calls
                .iter()
                .all(|(start, deadline)| *deadline == cutoff && start < deadline)
        );
    }
    .with_subscriber(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    )
    .await;
}

/// Waiting for another restore does not defer the cutoff or start Git after it expires.
#[tokio::test]
async fn deadline_expires_while_waiting_for_the_git_slot() {
    async {
        let world = World::new();
        let checkout = world.clone_into("locked-cutoff");
        let store = ObjectStore::start(fs::read(world.bundle()).unwrap(), &[]).await;
        let grants = DownloadGrants::new();
        controller(
            &grants,
            &store,
            world.prior.bundle.key.clone(),
            /*refuse*/ false,
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let restore = subject(
            &world,
            &grants,
            Arc::clone(&calls),
            DelayedDownload::new(Duration::ZERO),
            Duration::ZERO,
            RestorePolicy {
                deadline: Duration::from_secs(/*secs*/ 2),
                ..POLICY
            },
        );
        let _slot = restore.git.lock().await;
        let result = tokio::time::timeout(
            Duration::from_secs(/*secs*/ 4),
            restore.restore(request(&world, checkout)),
        )
        .await
        .unwrap();
        assert_eq!(result, Err(RestoreFailure::Unavailable));
        assert!(calls.lock().unwrap().is_empty());
        assert!(
            fs::read_dir(world.path("deadline-home").join(RESTORE_ROOT))
                .unwrap()
                .next()
                .is_none()
        );
    }
    .with_subscriber(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    )
    .await;
}

/// An interval that would cross the cutoff is rejected immediately instead of sleeping until it.
#[tokio::test]
async fn retry_backoff_does_not_wait_across_the_deadline() {
    async {
        let world = World::new();
        let checkout = world.clone_into("backoff-cutoff");
        let store = ObjectStore::start(fs::read(world.bundle()).unwrap(), &[500]).await;
        let grants = DownloadGrants::new();
        controller(
            &grants,
            &store,
            world.prior.bundle.key.clone(),
            /*refuse*/ false,
        );
        let restore = restorer(
            &grants,
            &world.path("backoff-home"),
            RestorePolicy {
                deadline: Duration::from_secs(/*secs*/ 2),
                backoff: Duration::from_secs(/*secs*/ 3),
                ..POLICY
            },
        );
        let result = tokio::time::timeout(
            Duration::from_secs(/*secs*/ 1),
            restore.restore(request(&world, checkout)),
        )
        .await
        .unwrap();
        assert_eq!(result, Err(RestoreFailure::Unavailable));
        assert_eq!(store.received().len(), 1);
    }
    .with_subscriber(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    )
    .await;
}
