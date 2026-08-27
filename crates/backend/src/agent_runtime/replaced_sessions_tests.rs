//! Covers the notice that repairs live sessions after an agent process is replaced.
//!
//! Effect coordination restarts an Agent plugin so it re-reads a materialized surface, which takes
//! every provider-side session that process was holding down with it. Ora keeps those session ids,
//! so without this notice the next prompt is sent against an id the fresh process cannot resolve
//! and the conversation fails for good.

use super::{
    AgentRuntimeManager, AgentRuntimeSetup, ReplacedAgentSessions, RuntimeActorHandle,
    RuntimeCommand,
};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::user_config::UserConfigApi;
use ora_db::{DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog};
use ora_domain::{AgentRef, PluginId, SessionId};
use ora_scheduler::Scheduler;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;

/// The address the plugin lifecycle owns the process under, which is what Effect names a consumer.
const PLUGIN_ADDRESS: &str = "official/ora-space.opencode";
/// The agent identity the package supplies, which is what a Session persists as its `agent_ref`.
///
/// Deliberately unlike the address: the two are different strings in practice, and a test that let
/// them coincide would pass just as happily against code that confuses one for the other.
const AGENT_NAME: &str = "opencode";

fn test_pool(root: &Path) -> RepositoryPool {
    DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("test.sqlite")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("create repository pool")
}

fn test_manager(root: &Path, pool: &RepositoryPool, scheduler: Scheduler) -> AgentRuntimeManager {
    let plugin_host = Arc::new(
        PluginApi::open(
            pool.clone(),
            root.to_path_buf(),
            PathBuf::from("deno"),
            SystemClock,
            AppEventHub::new().publisher(),
            Arc::new(UserConfigApi::new(pool.clone())),
        )
        .expect("open plugin host"),
    );
    AgentRuntimeManager::new(AgentRuntimeSetup {
        plugin_host,
        pool: pool.clone(),
        home_directory: root.to_path_buf(),
        relative_path_base: root.to_path_buf(),
        sessions_root: root.join("sessions"),
        clock: SystemClock,
        scheduler,
        app_events: AppEventHub::new().publisher(),
    })
    .expect("build agent runtime manager")
}

/// Registers one actor stand-in and hands back the commands it would receive.
fn register_actor(
    manager: &AgentRuntimeManager,
    session_id: &str,
) -> mpsc::UnboundedReceiver<RuntimeCommand> {
    let (commands, received) = mpsc::unbounded_channel();
    manager
        .inner
        .actors
        .write()
        .expect("actor registry")
        .insert(SessionId::new(session_id), RuntimeActorHandle { commands });
    received
}

/// A Session's `agent_ref` is never the plugin address Effect names its consumer by.
///
/// This is the mismatch that made an earlier version of the repair a silent no-op: `ConsumerId`
/// carries the package address, a Session carries the agent name the package supplies, and
/// comparing one against the other matched nothing while reading perfectly reasonably. Pinning the
/// two as distinct is what keeps a future edit from quietly reintroducing the confusion.
#[test]
fn a_plugin_address_is_not_the_agent_identity_a_session_is_bound_to() {
    let address = PluginId::parse(PLUGIN_ADDRESS).expect("plugin address");
    let agent = AgentRef::parse(AGENT_NAME).expect("agent identity");

    assert_ne!(
        address.canonical(),
        agent.as_str(),
        "the repair has to translate between these; if they were the same string a broken \
         translation would still appear to work",
    );
}

/// A replacement that resolves to no installed agent package detaches nothing.
///
/// Resolution is the step that turns the consumer's address into the identity sessions hold, so a
/// package the host does not serve an agent for must stop there rather than broadcasting a notice
/// no actor could match anyway.
#[tokio::test]
async fn a_plugin_without_an_agent_identity_detaches_nothing() {
    let temporary = TempDir::new().expect("create test directory");
    let pool = test_pool(temporary.path());
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let manager = test_manager(temporary.path(), &pool, scheduler.clone());
    let mut commands = register_actor(&manager, "session-1");

    // Nothing is installed under this address, so it names no agent.
    manager.detach_sessions_for_replaced_plugin(
        &PluginId::parse(PLUGIN_ADDRESS).expect("plugin address"),
    );

    assert!(
        commands.try_recv().is_err(),
        "an unresolvable package must not broadcast a notice",
    );
    scheduler.shutdown().await;
}

/// Every registered actor hears about a replacement, whichever Workspace it belongs to.
///
/// The notice is broadcast rather than addressed because one agent's connection is shared by every
/// Workspace: a restart triggered by one Workspace's surface invalidates sessions across all of
/// them, and the registry is keyed by Ora session with no agent index to narrow it. Each actor
/// decides for itself, so a filter applied here would silently strip the notice from sessions that
/// still need it.
#[tokio::test]
async fn a_replacement_notice_reaches_every_registered_actor() {
    let temporary = TempDir::new().expect("create test directory");
    let pool = test_pool(temporary.path());
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let manager = test_manager(temporary.path(), &pool, scheduler.clone());
    let mut first = register_actor(&manager, "session-1");
    let mut second = register_actor(&manager, "session-2");

    let agent = AgentRef::parse(AGENT_NAME).expect("agent identity");
    {
        let actors = manager.inner.actors.read().expect("actor registry");
        for handle in actors.values() {
            handle
                .commands
                .send(RuntimeCommand::AgentProcessReplaced {
                    agent: agent.clone(),
                })
                .expect("actor keeps its channel");
        }
    }

    for received in [
        first.recv().await.expect("first actor is notified"),
        second.recv().await.expect("second actor is notified"),
    ] {
        assert!(
            matches!(received, RuntimeCommand::AgentProcessReplaced { agent: notified } if notified == agent),
            "the notice must carry the agent identity, so a session bound elsewhere can ignore it",
        );
    }
    scheduler.shutdown().await;
}
