use gitlancer::parse::commit::{parse_commit_id, parse_commit_response};
use gitlancer::parse::status::parse_status_v2;
use gitlancer::parse::worktree::parse_worktree_list;
use gitlancer::{RepoRoot, WorktreeKind};

/// Verifies commit metadata parsing returns the commit ID and summary from the readback payload.
#[test]
fn parse_commit_response_reads_commit_id_and_summary() {
    let response = parse_commit_response(
        "0123456789abcdef0123456789abcdef01234567\nfeat: add linked worktree support\n",
    )
    .expect("parse commit response");

    assert_eq!(
        response.commit_id.as_str(),
        "0123456789abcdef0123456789abcdef01234567",
        "commit parser should preserve the full object ID"
    );
    assert_eq!(
        response.summary, "feat: add linked worktree support",
        "commit parser should preserve the latest summary"
    );
}

/// Verifies commit ID parsing trims whitespace and returns the first non-empty line.
#[test]
fn parse_commit_id_reads_first_non_empty_line() {
    let commit_id =
        parse_commit_id("\n 0123456789abcdef0123456789abcdef01234567 \n").expect("parse commit id");

    assert_eq!(
        commit_id.as_str(),
        "0123456789abcdef0123456789abcdef01234567",
        "commit ID parser should trim the selected line"
    );
}

/// Verifies porcelain v2 status parsing preserves each NUL-delimited status record.
#[test]
fn parse_status_v2_splits_nul_delimited_records() {
    let entries = parse_status_v2(
        "? untracked.txt\0 1 M. N... 100644 100644 100644 abcdef abcdef tracked.txt\0",
    )
    .expect("parse status");

    assert_eq!(entries.len(), 2, "two status records should be returned");
    assert_eq!(
        entries[0].raw, "? untracked.txt",
        "the first raw entry should match the first record"
    );
    assert!(
        entries[1].raw.contains("tracked.txt"),
        "the second raw entry should preserve the tracked file name"
    );
}

/// Verifies worktree parsing stamps every record with the owning repository and linked-worktree kind.
#[test]
fn parse_worktree_list_marks_main_and_linked_worktrees() {
    let repo_root = RepoRoot::new("/tmp/repo");
    let output = "\
worktree /tmp/repo
HEAD 0123456789abcdef0123456789abcdef01234567
branch refs/heads/main

worktree /tmp/worktrees/feature-tree
HEAD 89abcdef0123456789abcdef0123456789abcdef
branch refs/heads/feature/runtime
";

    let worktrees = parse_worktree_list(&repo_root, output).expect("parse worktrees");

    assert_eq!(
        worktrees.len(),
        2,
        "main and linked worktrees should be returned"
    );
    assert!(
        matches!(worktrees[0].kind(), WorktreeKind::Main),
        "the repository checkout should be classified as the main worktree"
    );
    assert!(
        matches!(
            worktrees[1].kind(),
            WorktreeKind::Linked { name } if name == "feature-tree"
        ),
        "linked worktrees should derive a stable name from the checkout path"
    );
    assert_eq!(
        worktrees[1].repo_root().as_path(),
        repo_root.as_path(),
        "linked worktrees should retain the owning repository root"
    );
}
