use super::*;
use gitlancer::{GitCommand, GitIntent};
use std::io;

/// Requires the original branch and remote-tracking commit, not a same-named tag or later user commit.
pub(super) fn verify<W: WriteGuard>(
    runner: &ManagedGitRunner<W>,
    record: &CloneExecution,
    config: &CloneConfig,
) -> io::Result<Option<CommitId>> {
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
        let output = runner.inspect_clone(&command)?;
        if output.code != Some(0) {
            return Err(io::Error::other("local checkout facts unavailable"));
        }
        Ok(output.stdout.trim_end_matches('\n').to_owned())
    };
    if query(vec!["rev-parse".into(), "--is-bare-repository".into()])? != "false"
        || query(vec!["rev-parse".into(), "--is-shallow-repository".into()])? != "false"
        || std::path::Path::new(&query(vec![
            "rev-parse".into(),
            "--absolute-git-dir".into(),
        ])?) != git_dir
        || query(vec![
            "config".into(),
            "--get".into(),
            "remote.origin.url".into(),
        ])? != record.command.payload.spec.repository.as_str()
    {
        return Err(io::Error::other(
            "checkout shape or source differs from intent",
        ));
    }
    let branch = record.command.payload.spec.branch.as_str();
    let local = format!("refs/heads/{branch}");
    let reference = query(vec![
        "for-each-ref".into(),
        "--format=%(refname)".into(),
        local.clone(),
    ])?;
    if reference != local {
        return Ok(None);
    }
    if query(vec!["symbolic-ref".into(), "HEAD".into()])? != local {
        return Err(io::Error::other("checkout is not on the requested branch"));
    }
    let head = query(vec![
        "rev-parse".into(),
        "--verify".into(),
        "HEAD^{commit}".into(),
    ])?;
    let fetched = query(vec![
        "rev-parse".into(),
        "--verify".into(),
        format!("refs/remotes/origin/{branch}^{{commit}}"),
    ])?;
    if head != fetched
        || !query(vec![
            "status".into(),
            "--porcelain".into(),
            "--untracked-files=no".into(),
        ])?
        .is_empty()
    {
        return Err(io::Error::other(
            "checkout no longer matches acquired facts",
        ));
    }
    Ok(Some(CommitId::new(head)))
}
