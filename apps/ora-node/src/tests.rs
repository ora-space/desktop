#![allow(clippy::unwrap_used, clippy::expect_used)]
mod coordination;
mod lifecycle;
mod local_recovery;
mod recovery;
mod review;
mod support;
use super::*;
use pretty_assertions::assert_eq;

/// A temporary home models deployment without depending on the process HOME variable.
#[test]
fn injected_home_persists_node_but_not_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let config = || NodeConfig {
        home_directory: dir.path().to_path_buf(),
        identity: NodeIdentity::Discover,
        repositories: vec![],
    };
    let first = Node::open_with_dependencies(
        config(),
        gitlancer::Git::new(gitlancer::CliGitRunner),
        DurableWrites,
        support::FixedClock,
    )
    .unwrap();
    assert_eq!(first.home_directory(), dir.path());
    assert!(dir.path().join("ora-node.sqlite3").is_file());
    let identity = first.identity().clone();
    drop(first);
    let next = Node::open_with_dependencies(
        config(),
        gitlancer::Git::new(gitlancer::CliGitRunner),
        DurableWrites,
        support::FixedClock,
    )
    .unwrap();
    assert_eq!(next.node_id(), &identity.node_id);
    assert_ne!(next.identity().incarnation_id, identity.incarnation_id);
}
