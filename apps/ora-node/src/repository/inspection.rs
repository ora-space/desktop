use super::*;
use crate::managed::CloneHost;
use gitlancer::{GitCommand, GitIntent};
use std::io;

/// Confirms the checkout the workload sequence built (Node clone ADR D3) and returns its HEAD:
/// a plain, complete repository of its own on the Workspace branch, with no remote and a clean
/// tree. The caller compares the commit with the input's base commit.
pub(super) fn verify(
    host: &CloneHost,
    record: &CloneExecution,
    config: &CloneConfig,
) -> io::Result<CommitId> {
    let target = &record.target.path;
    let git_dir = target.join(".git");
    DirectoryIdentity::read(&git_dir)?;
    if fs::canonicalize(git_dir.join("objects"))? != git_dir.join("objects")
        || fs::symlink_metadata(git_dir.join("commondir")).is_ok()
    {
        return Err(io::Error::other(
            "checkout metadata points outside its owned directory",
        ));
    }
    if fs::symlink_metadata(git_dir.join("objects").join("info").join("alternates")).is_ok() {
        return Err(io::Error::other("checkout references external objects"));
    }
    let query = |args: Vec<String>| -> io::Result<String> {
        let mut command = GitCommand::new(
            target.clone(),
            args,
            config.environment().map_err(io::Error::other)?,
            GitIntent::ReadOnly,
        );
        config.constrain(&mut command);
        let output = host.inspect(&command)?;
        if output.code != Some(0) {
            return Err(io::Error::other("local checkout facts unavailable"));
        }
        Ok(output.stdout.trim_end_matches('\n').to_owned())
    };
    let local = format!(
        "refs/heads/{}",
        record.command.payload.spec.workspace_ref.branch()
    );
    if query(vec!["rev-parse".into(), "--is-bare-repository".into()])? != "false"
        || query(vec!["rev-parse".into(), "--is-shallow-repository".into()])? != "false"
        || std::path::Path::new(&query(vec![
            "rev-parse".into(),
            "--absolute-git-dir".into(),
        ])?) != git_dir
        || !query(vec!["remote".into()])?.is_empty()
        || !query(vec![
            "for-each-ref".into(),
            "--format=%(refname)".into(),
            "refs/remotes/".into(),
        ])?
        .is_empty()
        || query(vec![
            "for-each-ref".into(),
            "--format=%(refname)".into(),
            local.clone(),
        ])? != local
        || query(vec!["symbolic-ref".into(), "HEAD".into()])? != local
    {
        return Err(io::Error::other(
            "checkout shape differs from the snapshot sequence",
        ));
    }
    let head = query(vec![
        "rev-parse".into(),
        "--verify".into(),
        "HEAD^{commit}".into(),
    ])?;
    if !query(vec![
        "status".into(),
        "--porcelain".into(),
        "--untracked-files=no".into(),
    ])?
    .is_empty()
    {
        return Err(io::Error::other("checkout tree differs from its HEAD"));
    }
    Ok(CommitId::new(head))
}
