//! The Git steps of a restore against real repositories (Node restore ADR D3, D4).
use super::super::git::GitRestore;
use super::*;
use crate::session::{RestoreFailure, Restored};
use pretty_assertions::assert_eq;

/// Restores `world`'s prior Revision into `checkout` through the production Git steps.
fn restore(
    world: &World,
    checkout: &Path,
    prior: &PriorRevision,
) -> Result<Restored, RestoreFailure> {
    delivery_git().restore(&GitRestore {
        checkout,
        bundle: &world.bundle(),
        prior,
        scratch: "session",
        // Unprivileged tests can only hand the copy to themselves; that still runs the chown.
        owner: Some(std::os::unix::fs::MetadataExt::uid(
            &fs::metadata(checkout).unwrap(),
        )),
    })
}

/// The clone's branch ends at the prior final commit with a clean worktree holding the prior
/// run's committed and uncommitted work; `origin/main` stays at the clone's base.
#[test]
fn restores_the_prior_final_commit_onto_the_clone_branch() {
    let world = World::new();
    let checkout = world.clone_into("fresh");
    assert_eq!(
        restore(&world, &checkout, &world.prior),
        Ok(Restored::OnRemoteHistory)
    );
    let final_commit = world.prior.final_commit.as_str().to_owned();
    assert_eq!(
        state(&checkout),
        CheckoutState {
            head: "refs/heads/main".into(),
            commit: final_commit.clone(),
            status: String::new(),
            revision_ref: final_commit,
            origin_main: world.base.as_str().into(),
            restore_scratch: vec![],
        }
    );
    assert_eq!(
        (
            fs::read_to_string(checkout.join("work.txt")).unwrap(),
            fs::read_to_string(checkout.join("notes.txt")).unwrap(),
        ),
        ("committed work\n".into(), "uncommitted work\n".into())
    );
}

/// A base commit the fresh clone lacks is fetched once from `origin` by its ID; since the remote
/// branch no longer contains it, the restore reports the rewritten history for the Agent's note.
#[test]
fn fetches_a_missing_base_and_reports_the_rewritten_remote_history() {
    let world = World::new();
    world.rewrite_origin(/*keep_base*/ true);
    let checkout = world.clone_into("fresh");
    let rewritten = git(&checkout, &["rev-parse", "HEAD"]);
    assert_eq!(
        restore(&world, &checkout, &world.prior),
        Ok(Restored::Diverged {
            final_commit: world.prior.final_commit.clone(),
            base_commit: world.base.clone(),
            branch: "main".into(),
        })
    );
    let final_commit = world.prior.final_commit.as_str().to_owned();
    assert_eq!(
        state(&checkout),
        CheckoutState {
            head: "refs/heads/main".into(),
            commit: final_commit.clone(),
            status: String::new(),
            revision_ref: final_commit,
            origin_main: rewritten,
            restore_scratch: vec![],
        }
    );
}

/// A base commit `origin` no longer has is the permanent failure Cloud marks the Revision for;
/// the checkout stays exactly as cloned.
#[test]
fn an_unfetchable_base_is_base_unavailable_and_leaves_the_checkout() {
    let world = World::new();
    world.rewrite_origin(/*keep_base*/ false);
    let checkout = world.clone_into("fresh");
    let before = state(&checkout);
    assert_eq!(
        restore(&world, &checkout, &world.prior),
        Err(RestoreFailure::BaseUnavailable)
    );
    assert_eq!(state(&checkout), before);
}

/// A remote that cannot be reached says nothing about the base commits: the failure stays
/// retryable instead of making Cloud refuse the Revision for good.
#[test]
fn an_unreachable_origin_is_unavailable_not_base_unavailable() {
    let world = World::new();
    world.rewrite_origin(/*keep_base*/ false);
    let checkout = world.clone_into("fresh");
    fs::rename(world.origin(), world.path("moved-away")).unwrap();
    assert_eq!(
        restore(&world, &checkout, &world.prior),
        Err(RestoreFailure::Unavailable)
    );
}

/// A bundle whose head is not the prior final commit, or whose pack is damaged, is unavailable
/// rather than restored, and the checkout stays exactly as cloned.
#[test]
fn a_wrong_head_or_damaged_bundle_is_unavailable() {
    let world = World::new();
    let checkout = world.clone_into("fresh");
    let before = state(&checkout);
    let other_head = PriorRevision {
        final_commit: world.base.clone(),
        ..world.prior.clone()
    };
    assert_eq!(
        restore(&world, &checkout, &other_head),
        Err(RestoreFailure::Unavailable)
    );
    let mut bytes = fs::read(world.bundle()).unwrap();
    let length = bytes.len();
    bytes.truncate(length - 8);
    fs::write(world.bundle(), &bytes).unwrap();
    assert_eq!(
        restore(&world, &checkout, &world.prior),
        Err(RestoreFailure::Unavailable)
    );
    assert_eq!(state(&checkout), before);
}

/// After a restore, a delivery that added nothing is unchanged at the prior final commit, while
/// new work is bundled against the clone's base and so carries the prior Revision's commits too.
#[test]
fn deliveries_after_a_restore_are_unchanged_or_bundle_the_prior_work() {
    let world = World::new();
    let checkout = world.clone_into("fresh");
    restore(&world, &checkout, &world.prior).unwrap();
    let author = GitIdentity {
        name: "Session User".into(),
        email: "user@example.com".into(),
    };
    let revision_ref = RevisionRef::new("refs/ora/revisions/run-2");
    let request = |scratch| SnapshotRequest {
        checkout: &checkout,
        revision_ref: &revision_ref,
        base_commit: &world.base,
        prior_final_commit: Some(&world.prior.final_commit),
        author: &author,
        scratch,
    };
    assert_eq!(
        delivery_git().snapshot(&request("unchanged")).unwrap(),
        Snapshot::Unchanged {
            final_commit: world.prior.final_commit.clone(),
        }
    );

    fs::write(checkout.join("more.txt"), "second run\n").unwrap();
    let Snapshot::Changed {
        final_commit,
        bundle,
    } = delivery_git().snapshot(&request("changed")).unwrap()
    else {
        panic!("the second run changed the checkout");
    };
    // The new bundle restores onto a clone of the base alone, prior work included.
    let next = world.clone_into("next");
    let bundle = bundle.to_str().unwrap().to_owned();
    git(
        &next,
        &[
            "fetch",
            "-q",
            &bundle,
            "refs/ora/revisions/run-2:refs/ora/revisions/run-2",
        ],
    );
    git(&next, &["checkout", "-q", final_commit.as_str()]);
    assert_eq!(
        (
            fs::read_to_string(next.join("work.txt")).unwrap(),
            fs::read_to_string(next.join("notes.txt")).unwrap(),
            fs::read_to_string(next.join("more.txt")).unwrap(),
        ),
        (
            "committed work\n".into(),
            "uncommitted work\n".into(),
            "second run\n".into()
        )
    );
}
