mod preview;

pub use preview::{DiffFile, DiffFilesResponse};

use crate::domain::refs::CommitId;
use crate::domain::worktree::WorktreeHandle;
use crate::error::{GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMPORARY_GIT_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const MAX_DIFF_BYTES: usize = 10 * 1024 * 1024;
const MAX_DIFF_STDERR_BYTES: usize = 1024 * 1024;
// Full, compact, and single-file reads use the same transport budget.
const REVIEW_DIFF_BYTES: usize = 20 * 1024 * 1024;
const COMPACT_CONTEXT_ARGS: &[&str] = &["--unified=3", "--inter-hunk-context=0"];
const FULL_FILE_CONTEXT_ARGS: &[&str] = &["--unified=1000000"];

/// Selects the Git layer represented by one task diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffScope {
    Branch,
    Unstaged,
    Staged,
    Committed,
}

/// Carries the worktree, comparison scope, and baseline, when the caller has one recorded.
///
/// `Unstaged` and `Staged` never read `base_commit_id`. `Branch` and `Committed` require it —
/// the caller must not request those scopes without a recorded baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub base_commit_id: Option<&'a CommitId>,
    pub scope: DiffScope,
}

/// Returns a standard unified patch that frontend diff parsers can consume directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffResponse {
    pub head_commit_id: CommitId,
    pub patch: String,
}

impl<R: GitRunner> Git<R> {
    /// Reads a full-context workspace review patch within the review budget.
    pub fn diff(&self, request: DiffRequest<'_>) -> Result<DiffResponse, GitlancerError> {
        self.read_patch(request, &[], FULL_FILE_CONTEXT_ARGS, REVIEW_DIFF_BYTES)
    }

    /// Computes the same diff while ignoring every path under the excluded prefixes.
    ///
    /// The Scheduled Run artifact directory is a declared output, not a worktree change, so
    /// retention must still see the worktree as unmodified even when the agent committed it.
    pub fn diff_excluding_paths(
        &self,
        request: DiffRequest<'_>,
        excluded_paths: &[&str],
    ) -> Result<DiffResponse, GitlancerError> {
        self.read_patch(
            request,
            excluded_paths,
            FULL_FILE_CONTEXT_ARGS,
            MAX_DIFF_BYTES,
        )
    }

    /// Reads the same comparison with ordinary unified context for the review view.
    pub fn diff_compact(&self, request: DiffRequest<'_>) -> Result<DiffResponse, GitlancerError> {
        self.read_patch(request, &[], COMPACT_CONTEXT_ARGS, REVIEW_DIFF_BYTES)
    }

    /// Keeps patch generation and byte accounting identical for both context modes.
    fn read_patch(
        &self,
        request: DiffRequest<'_>,
        excluded_paths: &[&str],
        context: &[&str],
        max_bytes: usize,
    ) -> Result<DiffResponse, GitlancerError> {
        let head_commit_id = self.diff_head(request.worktree)?;

        let tracked = self
            .runner()
            .run_bounded(
                &build_tracked_diff_command(&request, excluded_paths, context, "HEAD"),
                max_bytes,
                MAX_DIFF_STDERR_BYTES,
            )
            .map_err(|error| map_bounded_diff_error(error, max_bytes))?
            .stdout;
        let mut patch = tracked;
        let untracked_output = self
            .runner()
            .run_bounded(
                &build_untracked_command(request.worktree, excluded_paths),
                MAX_DIFF_BYTES,
                MAX_DIFF_STDERR_BYTES,
            )
            .map_err(|error| map_bounded_diff_error(error, max_bytes))?;
        let isolated_git_dir = isolated_git_dir();

        for path in untracked_output
            .stdout
            .split('\0')
            .filter(|path| !path.is_empty())
            .filter(|_| matches!(request.scope, DiffScope::Branch | DiffScope::Unstaged))
        {
            self.append_untracked(
                request.worktree,
                path,
                &isolated_git_dir,
                context,
                max_bytes,
                &mut patch,
            )?;
        }

        Ok(DiffResponse {
            head_commit_id,
            patch,
        })
    }
    /// Resolves the HEAD label shared by every diff output.
    fn diff_head(&self, worktree: &WorktreeHandle) -> Result<CommitId, GitlancerError> {
        let output = self.runner().run(&build_head_command(worktree))?;
        let head = output.stdout.trim();
        if head.is_empty() {
            return Err(crate::ParseError::MissingLine.into());
        }
        Ok(CommitId::new(head))
    }

    /// Adds a single untracked file within the response's remaining byte budget.
    fn append_untracked(
        &self,
        worktree: &WorktreeHandle,
        path: &str,
        git_dir: &std::path::Path,
        context: &[&str],
        max_bytes: usize,
        patch: &mut String,
    ) -> Result<(), GitlancerError> {
        let separator_bytes = usize::from(!patch.is_empty() && !patch.ends_with('\n'));
        let remaining = max_bytes.saturating_sub(patch.len() + separator_bytes);
        let addition = run_untracked_diff(
            self.runner(),
            &build_untracked_diff_command(worktree, path, git_dir, context),
            remaining,
        )
        .map_err(|error| map_bounded_diff_error(error, max_bytes))?;
        let addition = if addition.is_empty() {
            run_empty_untracked_diff(self.runner(), worktree, path, remaining, context)
                .map_err(|error| map_bounded_diff_error(error, max_bytes))?
        } else {
            addition
        };
        append_patch(patch, &addition);
        Ok(())
    }
}

/// Generates a process-unique nonexistent Git directory so no-index ignores repository filters.
fn isolated_git_dir() -> std::path::PathBuf {
    // The path is intentionally never created: Git treats it as no repository, so there is
    // no temporary directory or cleanup guard to maintain for this read-only comparison.
    let sequence = TEMPORARY_GIT_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let process_id = std::process::id();
    std::env::temp_dir().join(format!("ora-no-index-git-dir-{process_id}-{sequence}"))
}

/// Owns a temporary Git index path and removes any index or lock file left by Git commands.
struct TemporaryIndex {
    path: std::path::PathBuf,
}

impl TemporaryIndex {
    /// Reserves a process-unique path without creating an invalid empty index file.
    fn new() -> Self {
        let sequence = TEMPORARY_GIT_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let process_id = std::process::id();
        Self {
            path: std::env::temp_dir()
                .join(format!("ora-empty-untracked-index-{process_id}-{sequence}")),
        }
    }

    /// Returns the path passed to Git through `GIT_INDEX_FILE`.
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TemporaryIndex {
    /// Best-effort cleanup keeps diff failures from accumulating temporary index files.
    fn drop(&mut self) {
        let _remove_index_result = std::fs::remove_file(&self.path);
        let lock_path = self.path.with_extension("lock");
        let _remove_lock_result = std::fs::remove_file(lock_path);
    }
}

/// Accepts normal difference and TOCTOU disappearance exits from `git diff --no-index`.
fn run_untracked_diff<R: GitRunner>(
    runner: &R,
    command: &GitCommand,
    max_stdout_bytes: usize,
) -> Result<String, GitExecError> {
    match runner.run_bounded(command, max_stdout_bytes, MAX_DIFF_STDERR_BYTES) {
        Ok(output) => Ok(output.stdout),
        Err(GitExecError::NonZeroExit {
            code: Some(1),
            stdout,
            ..
        }) => Ok(stdout),
        Err(GitExecError::NonZeroExit {
            code: Some(128),
            stdout,
            stderr,
            ..
        }) if is_missing_untracked_file_error(&stderr) => Ok(stdout),
        Err(error) => Err(error),
    }
}

/// Identifies the Git diagnostics produced when a listed untracked file disappears before comparison.
fn is_missing_untracked_file_error(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("no such file or directory")
        || stderr.contains("does not exist")
        || (stderr.contains("pathspec") && stderr.contains("did not match"))
}

/// Uses an isolated intent-to-add index so Git emits metadata for an empty untracked file.
fn run_empty_untracked_diff<R: GitRunner>(
    runner: &R,
    worktree: &WorktreeHandle,
    path: &str,
    max_stdout_bytes: usize,
    context: &[&str],
) -> Result<String, GitExecError> {
    let temporary_index = TemporaryIndex::new();
    runner.run(&build_initialize_temporary_index_command(
        worktree,
        temporary_index.path(),
    ))?;
    runner.run(&build_intent_to_add_command(
        worktree,
        path,
        temporary_index.path(),
    ))?;
    runner
        .run_bounded(
            &build_empty_untracked_diff_command(worktree, path, temporary_index.path(), context),
            max_stdout_bytes,
            MAX_DIFF_STDERR_BYTES,
        )
        .map(|output| output.stdout)
}

/// Converts bounded runner failures into the public diff-size error when appropriate.
fn map_bounded_diff_error(error: GitExecError, max_bytes: usize) -> GitlancerError {
    match error {
        GitExecError::OutputTooLarge {
            stream: "stdout", ..
        } => GitlancerError::DiffTooLarge {
            byte_count: max_bytes + 1,
            max_byte_count: max_bytes,
        },
        error => GitlancerError::Exec(error),
    }
}

/// Adds a file patch while preserving exactly one separator between patch streams.
fn append_patch(patch: &mut String, addition: &str) {
    if addition.is_empty() {
        return;
    }
    if !patch.is_empty() && !patch.ends_with('\n') {
        patch.push('\n');
    }
    patch.push_str(addition);
}

/// Builds the HEAD lookup command so uncommitted changes still carry their branch revision.
fn build_head_command(worktree: &WorktreeHandle) -> GitCommand {
    command(worktree, vec!["rev-parse", "HEAD"])
}

/// Builds the tracked-file comparison without external diff or text-conversion processes.
pub fn build_diff_command(request: &DiffRequest<'_>) -> GitCommand {
    build_diff_command_excluding(request, &[])
}

/// Builds the tracked-file comparison while excluding every path below the given prefixes.
pub fn build_diff_command_excluding(
    request: &DiffRequest<'_>,
    excluded_paths: &[&str],
) -> GitCommand {
    build_tracked_diff_command(request, excluded_paths, FULL_FILE_CONTEXT_ARGS, "HEAD")
}

/// Selects the tracked diff output before Git interprets revisions and pathspecs.
fn build_tracked_diff_command(
    request: &DiffRequest<'_>,
    excluded_paths: &[&str],
    output: &[&str],
    head_revision: &str,
) -> GitCommand {
    // Render paths verbatim: with the default `core.quotepath`, Git octal-escapes
    // non-ASCII (e.g. Chinese) names in `diff --git` headers, which the frontend diff
    // parser does not decode back — jump-to-file would break for those paths.
    let mut args = vec![
        "-c".to_string(),
        "core.quotepath=false".to_string(),
        "diff".to_string(),
        "--no-color".to_string(),
        "--no-ext-diff".to_string(),
        "--no-textconv".to_string(),
        "--find-renames".to_string(),
    ];
    args.extend(output.iter().map(|arg| (*arg).to_string()));
    match request.scope {
        DiffScope::Branch => {
            args.push(expect_baseline(request.base_commit_id).as_str().to_string());
        }
        DiffScope::Unstaged => {}
        DiffScope::Staged => {
            args.push("--cached".to_string());
            args.push(head_revision.to_string());
        }
        DiffScope::Committed => {
            args.push(expect_baseline(request.base_commit_id).as_str().to_string());
            args.push(head_revision.to_string());
        }
    }
    args.push("--".to_string());
    append_excluded_pathspecs(&mut args, excluded_paths);
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

/// Adds an include-all pathspec plus one recursive exclusion for every requested prefix.
///
/// Git accepts exclude-only pathspecs in some versions, but pairing them with an explicit
/// `.` keeps the intent unambiguous: include everything below the worktree root, except the
/// declared output tree.
fn append_excluded_pathspecs(args: &mut Vec<String>, excluded_paths: &[&str]) {
    if excluded_paths.is_empty() {
        return;
    }
    args.push(".".to_string());
    for path in excluded_paths {
        args.push(format!(":(exclude){path}"));
    }
}

/// Reads the baseline required by the `Branch`/`Committed` scopes.
///
/// Panics on `None`: the application layer must reject those scopes before a request without a
/// recorded baseline ever reaches this command builder.
fn expect_baseline(base_commit_id: Option<&CommitId>) -> &CommitId {
    base_commit_id.expect("Branch/Committed diff scope requires a recorded baseline commit")
}

/// Lists ignored-aware untracked paths in a machine-readable representation.
fn build_untracked_command(worktree: &WorktreeHandle, excluded_paths: &[&str]) -> GitCommand {
    let mut args = vec![
        "ls-files".to_string(),
        "--others".to_string(),
        "--exclude-standard".to_string(),
        "-z".to_string(),
    ];
    if !excluded_paths.is_empty() {
        args.push("--".to_string());
        append_excluded_pathspecs(&mut args, excluded_paths);
    }
    GitCommand::new(
        worktree.worktree_root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

/// Lets Git render one untracked file with correct quoting, modes, symlinks, and binary markers.
fn build_untracked_diff_command(
    worktree: &WorktreeHandle,
    path: &str,
    isolated_git_dir: &std::path::Path,
    context: &[&str],
) -> GitCommand {
    GitCommand::new(
        worktree.worktree_root().as_path().to_path_buf(),
        vec![
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
        ]
        .into_iter()
        .chain(context.iter().copied())
        .chain(["--", "/dev/null", path])
        .map(str::to_string)
        .collect(),
        // `run_untracked_diff` recognizes a vanished file by Git's English stderr.
        GitEnv::default()
            .with_english_messages()
            .with_variable("GIT_DIR", isolated_git_dir.to_string_lossy().into_owned()),
        GitIntent::ReadOnly,
    )
}

/// Initializes a temporary index from HEAD without reading or changing the real worktree index.
fn build_initialize_temporary_index_command(
    worktree: &WorktreeHandle,
    temporary_index: &std::path::Path,
) -> GitCommand {
    command_with_index(
        worktree,
        vec!["read-tree", "HEAD"],
        temporary_index,
        GitIntent::Mutating,
    )
}

/// Records only intent-to-add metadata so Git can distinguish an empty file from `/dev/null`.
fn build_intent_to_add_command(
    worktree: &WorktreeHandle,
    path: &str,
    temporary_index: &std::path::Path,
) -> GitCommand {
    command_with_index(
        worktree,
        vec!["--literal-pathspecs", "add", "--intent-to-add", "--", path],
        temporary_index,
        GitIntent::Mutating,
    )
}

/// Renders an empty intent-to-add entry as a canonical new-file patch.
fn build_empty_untracked_diff_command(
    worktree: &WorktreeHandle,
    path: &str,
    temporary_index: &std::path::Path,
    context: &[&str],
) -> GitCommand {
    command_with_index(
        worktree,
        vec![
            "-c",
            "core.quotepath=false",
            "--literal-pathspecs",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
        ]
        .into_iter()
        .chain(context.iter().copied())
        .chain(["--", path])
        .collect(),
        temporary_index,
        GitIntent::ReadOnly,
    )
}

/// Creates a Git command whose mutations are isolated to a disposable index file.
fn command_with_index(
    worktree: &WorktreeHandle,
    args: Vec<&str>,
    temporary_index: &std::path::Path,
    intent: GitIntent,
) -> GitCommand {
    GitCommand::new(
        worktree.worktree_root().as_path().to_path_buf(),
        args.into_iter().map(str::to_string).collect(),
        GitEnv::default().with_variable(
            "GIT_INDEX_FILE",
            temporary_index.to_string_lossy().into_owned(),
        ),
        intent,
    )
}

/// Creates a read-only Git command from borrowed arguments.
fn command(worktree: &WorktreeHandle, args: Vec<&str>) -> GitCommand {
    GitCommand::new(
        worktree.worktree_root().as_path().to_path_buf(),
        args.into_iter().map(str::to_string).collect(),
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        DiffRequest, DiffScope, FULL_FILE_CONTEXT_ARGS, build_diff_command,
        build_diff_command_excluding, build_empty_untracked_diff_command,
        build_initialize_temporary_index_command, build_intent_to_add_command,
        build_untracked_diff_command, is_missing_untracked_file_error,
    };
    use crate::{CommitId, GitDir, RepoRoot, WorktreeHandle, WorktreeKind, WorktreeRoot};
    use pretty_assertions::assert_eq;

    /// Verifies tracked diffs disable executable filters and emit parser-friendly binary markers.
    #[test]
    fn builds_task_diff_command() {
        let worktree = test_worktree();
        let base_commit_id = CommitId::new("base-commit");
        let command = build_diff_command(&DiffRequest {
            worktree: &worktree,
            base_commit_id: Some(&base_commit_id),
            scope: DiffScope::Branch,
        });

        assert_eq!(
            command.args,
            vec![
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames",
                "--unified=1000000",
                "base-commit",
                "--",
            ]
        );
    }

    /// Verifies the committed comparison excludes the declared output tree while still
    /// including everything else below the worktree root.
    #[test]
    fn builds_diff_command_with_excluded_artifact_directory() {
        let worktree = test_worktree();
        let base_commit_id = CommitId::new("base-commit");
        let command = build_diff_command_excluding(
            &DiffRequest {
                worktree: &worktree,
                base_commit_id: Some(&base_commit_id),
                scope: DiffScope::Committed,
            },
            &[".ora-scheduled-artifacts"],
        );

        assert_eq!(
            command.args,
            vec![
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames",
                "--unified=1000000",
                "base-commit",
                "HEAD",
                "--",
                ".",
                ":(exclude).ora-scheduled-artifacts",
            ]
        );
    }

    /// Verifies an unstaged diff never requires a baseline, since a workspace with no recorded
    /// baseline (a project's main checkout) must still be able to request one.
    #[test]
    fn builds_unstaged_diff_command_without_a_baseline() {
        let worktree = test_worktree();
        let command = build_diff_command(&DiffRequest {
            worktree: &worktree,
            base_commit_id: None,
            scope: DiffScope::Unstaged,
        });

        assert_eq!(
            command.args,
            vec![
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames",
                "--unified=1000000",
                "--",
            ]
        );
    }

    /// Verifies the `Branch` scope refuses to silently drop the comparison when no baseline was
    /// supplied, rather than emitting a `git diff` command missing its comparison target.
    #[test]
    #[should_panic(expected = "requires a recorded baseline commit")]
    fn panics_building_branch_diff_command_without_a_baseline() {
        let worktree = test_worktree();
        let _ = build_diff_command(&DiffRequest {
            worktree: &worktree,
            base_commit_id: None,
            scope: DiffScope::Branch,
        });
    }

    /// Verifies untracked files use Git's no-index renderer without clean or textconv filters.
    #[test]
    fn builds_untracked_file_diff_command() {
        let worktree = test_worktree();

        assert_eq!(
            build_untracked_diff_command(
                &worktree,
                "space name.bin",
                std::path::Path::new("/tmp/missing-git-dir"),
                FULL_FILE_CONTEXT_ARGS,
            )
            .args,
            vec![
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--unified=1000000",
                "--",
                "/dev/null",
                "space name.bin",
            ]
        );
    }

    /// Verifies empty-file fallback commands share one isolated index and never touch the real one.
    #[test]
    fn builds_empty_untracked_file_commands() {
        let worktree = test_worktree();
        let temporary_index = std::path::Path::new("/tmp/empty-file-index");

        let commands = [
            build_initialize_temporary_index_command(&worktree, temporary_index),
            build_intent_to_add_command(&worktree, "empty.txt", temporary_index),
            build_empty_untracked_diff_command(
                &worktree,
                "empty.txt",
                temporary_index,
                FULL_FILE_CONTEXT_ARGS,
            ),
        ];

        assert_eq!(
            commands.map(|command| (command.args, command.env.variables)),
            [
                (
                    vec!["read-tree".to_string(), "HEAD".to_string()],
                    [(
                        "GIT_INDEX_FILE".to_string(),
                        "/tmp/empty-file-index".to_string(),
                    ),]
                    .into(),
                ),
                (
                    vec![
                        "--literal-pathspecs".to_string(),
                        "add".to_string(),
                        "--intent-to-add".to_string(),
                        "--".to_string(),
                        "empty.txt".to_string(),
                    ],
                    [(
                        "GIT_INDEX_FILE".to_string(),
                        "/tmp/empty-file-index".to_string(),
                    ),]
                    .into(),
                ),
                (
                    vec![
                        "-c".to_string(),
                        "core.quotepath=false".to_string(),
                        "--literal-pathspecs".to_string(),
                        "diff".to_string(),
                        "--no-color".to_string(),
                        "--no-ext-diff".to_string(),
                        "--no-textconv".to_string(),
                        "--unified=1000000".to_string(),
                        "--".to_string(),
                        "empty.txt".to_string(),
                    ],
                    [(
                        "GIT_INDEX_FILE".to_string(),
                        "/tmp/empty-file-index".to_string(),
                    ),]
                    .into(),
                ),
            ]
        );
    }

    /// Accepts the platform-specific diagnostics emitted when a listed file is deleted mid-read.
    #[test]
    fn recognizes_missing_untracked_file_diagnostics() {
        assert_eq!(
            is_missing_untracked_file_error("fatal: pathspec 'gone.txt' did not match any files"),
            true
        );
        assert_eq!(
            is_missing_untracked_file_error(
                "fatal: unable to read file: No such file or directory"
            ),
            true
        );
        assert_eq!(
            is_missing_untracked_file_error("fatal: bad repository"),
            false
        );
    }

    /// Builds a linked worktree fixture without touching the filesystem.
    fn test_worktree() -> WorktreeHandle {
        WorktreeHandle::new(
            RepoRoot::new("/repo"),
            WorktreeRoot::new("/repo/worktrees/task"),
            GitDir::new("/repo/.git/worktrees/task"),
            WorktreeKind::Linked {
                name: "task".to_string(),
            },
            None,
        )
    }
}
