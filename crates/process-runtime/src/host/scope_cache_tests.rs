//! The in-memory Scope set must stay equal to the journal it replaces on the tick path.
use super::*;
use ora_process_protocol::{CleanupEvidence, ScopeState};
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Host state lives under HOME so its socket paths stay within the platform length limit.
fn state_directory() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
    Ok(tempfile::Builder::new()
        .prefix("sc")
        .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(home)?)
}

/// Reads the journal's own enumeration, which is what tick used before the cache existed.
fn journal_scopes(host: &HostCoordinator) -> Result<BTreeSet<ScopeId>, ProcessStateError> {
    Ok(host
        .state
        .scope_intents()?
        .into_iter()
        .map(|intent| intent.scope)
        .collect())
}

/// Creation appends exactly the accepted Scopes, and a replayed creation adds nothing.
#[tokio::test]
async fn created_scopes_match_the_journal() -> TestResult {
    let directory = state_directory()?;
    let mut host = HostCoordinator::new(
        HostState::create(&directory.path().join("s"))?,
        directory.path().join("unused"),
    )?;
    assert_eq!(host.scopes, BTreeSet::new());
    let scopes: BTreeSet<_> = (0..3).map(|_| ScopeId::new()).collect();
    for scope in &scopes {
        host.create_scope(*scope)?;
        host.create_scope(*scope)?;
    }
    assert_eq!(host.scopes, scopes);
    assert_eq!(host.scopes, journal_scopes(&host)?);
    Ok(())
}

/// A recovered host has no memory of earlier creations; it must reload them to keep serving them.
#[tokio::test]
async fn recovery_rebuilds_scopes_from_the_journal() -> TestResult {
    let directory = state_directory()?;
    let root = directory.path().join("s");
    let mut host =
        HostCoordinator::new(HostState::create(&root)?, directory.path().join("unused"))?;
    let scopes: BTreeSet<_> = (0..3).map(|_| ScopeId::new()).collect();
    for scope in &scopes {
        host.create_scope(*scope)?;
    }
    drop(host);
    let mut host =
        HostCoordinator::new(HostState::recover(&root)?, directory.path().join("unused"))?;
    assert_eq!(host.scopes, scopes);
    let fresh = ScopeId::new();
    host.create_scope(fresh)?;
    assert_eq!(host.scopes, journal_scopes(&host)?);
    // The reloaded Scopes are live responsibilities again, not just listed.
    host.tick()?;
    assert_eq!(host.active, journal_scopes(&host)?);
    Ok(())
}

/// Once a Scope is settled, later passes skip it instead of opening another guardian exchange.
#[tokio::test]
async fn settled_scope_is_not_dispatched_again() -> TestResult {
    let directory = state_directory()?;
    let mut host = HostCoordinator::new(
        HostState::create(&directory.path().join("s"))?,
        directory.path().join("unused"),
    )?;
    let scope = ScopeId::new();
    host.create_scope(scope)?;
    host.close(scope)?;
    host.state.observe_scope(
        scope,
        ScopeState::Closed(CleanupEvidence::BestEffortComplete),
    )?;
    host.progress.insert(
        scope,
        Progress {
            status: HostCoordination::Observing,
            retry_at: Instant::now(),
            failures: 0,
            control_pending: false,
            settled: false,
        },
    );
    for _ in 0..3 {
        host.tick()?;
        assert!(host.progress[&scope].settled);
        assert_eq!(host.active, BTreeSet::new());
    }
    Ok(())
}
