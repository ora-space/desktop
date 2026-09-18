use crate::{GitCommand, GitEnv, GitIntent};
use std::path::Path;

/// Builds a complete-history, single-branch checkout without templates, submodules or local object sharing.
/// The caller supplies validated source/branch and owns environment, hooks and credential policies.
pub fn build_branch_clone_command(
    source: &str,
    branch: &str,
    destination: &Path,
    working_dir: &Path,
    env: GitEnv,
) -> GitCommand {
    GitCommand::new(
        working_dir.to_path_buf(),
        vec![
            "clone".into(),
            "--no-local".into(),
            "--reject-shallow".into(),
            "--single-branch".into(),
            "--no-tags".into(),
            "--no-recurse-submodules".into(),
            "--template=".into(),
            "--origin".into(),
            "origin".into(),
            "--branch".into(),
            branch.into(),
            "--".into(),
            source.into(),
            destination.to_string_lossy().into_owned(),
        ],
        env,
        GitIntent::Network,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Policy contains no depth, alternate object store or caller-built shell command.
    #[test]
    fn explicit_branch_clone_has_fixed_nonlocal_full_history_arguments() {
        let root = std::env::temp_dir();
        let target = root.join("checkout");
        assert_eq!(
            build_branch_clone_command(
                "https://example.com/repo",
                "feature/one",
                &target,
                &root,
                GitEnv::default()
            ),
            GitCommand::new(
                root,
                vec![
                    "clone",
                    "--no-local",
                    "--reject-shallow",
                    "--single-branch",
                    "--no-tags",
                    "--no-recurse-submodules",
                    "--template=",
                    "--origin",
                    "origin",
                    "--branch",
                    "feature/one",
                    "--",
                    "https://example.com/repo",
                    &target.to_string_lossy()
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                GitEnv::default(),
                GitIntent::Network
            )
        );
    }
}
