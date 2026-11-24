# Gitlancer

[English](gitlancer.md) | 中文

`gitlancer`（`crates/gitlancer`）是 Ora 的类型化 Git 运行时。它构建在 Git CLI 之上（不用 `libgit2`），把多 worktree 作为一等的形态，并给上层提供类型化的请求和响应，而不是原始参数。它不管理 Ora 的记录，不在 Git 失败后清理数据库状态，不决定确认策略，也不重试命令；任务和 worktree 的策略在 `ora-application` 中（见[任务与 worktree](tasks-and-worktrees.zh.md)）。

## 分层

| 模块     | 负责                                                                                                                                                 |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `domain` | 仓库事实和不变量：`RepoRoot`、`WorktreeRoot`、`GitDir`、`RepoRelativePath`、`BranchName`、`CommitId`、`Repository`、`WorktreeHandle`、`WorktreeKind` |
| `exec`   | CLI 调用：`GitCommand`、`GitIntent`、`GitEnv`、`GitOutput`、`GitRunner`、`CliGitRunner`、`RecordingGitRunner`                                        |
| `git`    | Ora 直接调用的类型化用例                                                                                                                             |
| `parse`  | 把稳定的 porcelain 和 plumbing 输出转成类型化结果                                                                                                    |

`domain` 不启动进程也不解析输出。`WorktreeHandle::resolve_repo_relative_path` 对路径做词法规范化，拒绝绝对路径和逃出 worktree 的遍历，所以 `RepoRelativePath` 只能通过这个感知 worktree 的边界获得。根目录的构造函数假定调用方已经确认过 Git 身份；发现属于 `git`。

`Git<R: GitRunner>` 是所有用例的入口：它拥有执行策略，但不持有可变的仓库状态；静态分发让 runner 可以在测试中注入。

```rust
pub struct WorktreeHandle {
    repo_root: RepoRoot,
    worktree_root: WorktreeRoot,
    git_dir: GitDir,
    kind: WorktreeKind, // Main | Linked { name }
    branch_name: Option<BranchName>,
}
```

这区分了仓库根、命令运行的目录和该 worktree 背后的 gitdir。每个用例都在 `Repository` 或 `WorktreeHandle` 的上下文中执行。

## 用例

- **仓库与 worktree**：`discover_repository` 从 `git worktree list --porcelain` 读取 main checkout（而不是用 `rev-parse --show-toplevel`），所以从嵌套目录或链接 worktree 内部发现，都会解析到所属的仓库。`list_worktrees` 为每个 checkout 返回一个句柄，其中恰好一个是 `WorktreeKind::Main`。`resolve_worktree` 按名称查找链接 worktree，`resolve_worktree_by_branch` 按检出的分支查找（Ora 就是这样找回任务 checkout 的），`find_worktree` 查找包含某个嵌套路径的 worktree。
- **分支与基础**：`list_branches` 返回本地分支。`list_worktree_bases` 读取本地 `refs/heads`，`resolve_worktree_base_commit` 把选中的 ref 解析为 `CommitId`，没有网络或仓库状态的副作用。
- **生命周期**：`create_branch` 和 `create_worktree` 在调用方给出的 `CommitId` 上执行；`delete_branch` 和 `delete_worktree` 需要显式的 checked 或 forced 删除模式。
- **评审**：`diff` 为 branch、unstaged、staged 或 committed 范围返回完整上下文的 unified patch，渲染未跟踪文件时不改动调用方的 index；`status` 为 `git status --porcelain=v2 -z` 的每条记录返回一个条目；`add` 暂存 `RepoRelativePath`；`commit` 创建不签名的提交，并返回新的 `HEAD` 和摘要。
- **网络**：`push_branch` 把从 `WorktreeHandle` 推导出的分支推送到 `origin`，并禁用交互提示。同步操作负责 clone、fetch、checkout 和 fast-forward，marketplace 源使用它们。
- **检查点**：通过临时 index 把 worktree 快照到 `refs/ora/checkpoints/<name>`，列出自该 commit 以来改动的路径，并恢复选定的或全部路径，不碰调用方的 index。工作流运行使用它们，见[工作流运行](../workflows/workflow-runs.zh.md#回滚)。
- **身份**：`read_global_identity` 返回名称和邮箱，未设置的 key 视为 `None`。

只要非法状态能从仓库元数据看出来，变更操作就会在调用 Git 之前完成校验，校验失败时不发出任何命令：删除 main worktree（`MainWorktreeDeletionUnsupported`）、删除另一个仓库的 worktree（`WorktreeMismatch`）、创建已存在的分支或删除不存在的分支（`BranchAlreadyExists`、`BranchNotFound`）。

请求是显式的对象，而不是可变句柄上的方法，这样便于记录日志、扩展和校验：

```rust
pub struct CreateWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_root: WorktreeRoot,
    pub branch_name: BranchName,
    pub base_commit_id: CommitId,
}
```

## 执行

```rust
pub struct GitCommand {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: GitEnv,
    pub intent: GitIntent, // ReadOnly | Mutating | Network
}
```

`GitIntent` 只分类不强制；上层用它决定命令能否自动运行、是否需要确认、能否重试。`GitEnv::automation_defaults` 禁用终端提示，固定 `LANG=C` 并禁用分页，所以 Agent 驱动的命令不会卡在交互界面上。调用方设置的 `LC_ALL` 仍会覆盖 `LANG`；按文本分类 stderr 的命令（工作树探测、未跟踪文件的 diff）会加上 `GitEnv::with_english_messages`，只为该命令固定 `LC_ALL=C`。它不作为默认值，因为 `LC_ALL` 也会覆盖提交和推送时运行的 hook 的 `LC_CTYPE`。

`CliGitRunner` 运行系统的 `git`，测量耗时，报告可选的遥测，返回 `GitOutput { code, stdout, stderr, duration_ms }`。`run_bounded` 并发地读取 stdout 和 stderr，任一流超出预算就杀掉 Git（`GitExecError::OutputTooLarge`，diff 中映射为 `GitlancerError::DiffTooLarge`）。`run_within(command, limit)` 是按次选择开启的期限，目前只有工作树探测使用。`CliGitRunner` 在超过期限后杀掉 Git，返回带 `ErrorKind::TimedOut` 的 `OutputReadFailed`；期限也覆盖 Git 退出后孙进程仍占着输出管道时的等待，这样的孙进程会继续运行。它不限制回收操作系统暂时无法终止的已杀进程（不可中断的文件系统等待），也不限制调用方在命令前后做的事情，例如探测里对 `.git` 的 `stat` 检查。其他 runner 会把期限当作命令的 `time_limit` 收到，有执行预算的 runner 会遵守它。非零退出会保留退出码、参数和输出供诊断。`RecordingGitRunner` 不执行任何东西，用于测试命令构造。遥测通过 `gitlancer::logging::register` 选择开启，见[日志](../core/logging.zh.md#git-命令日志)。

## 解析

只解析稳定的机器可读输出：`git worktree list --porcelain`、`git for-each-ref refs/heads`、`git rev-parse <ref>^{commit}`、`git status --porcelain=v2 -z`，以及提交后的 `git rev-parse HEAD` 和 `git log -1 --pretty=%s`。空的或结构不完整的载荷是 `ParseError`；解析器从不编造身份。detached 的 worktree 没有分支。给人看的 stderr 留作诊断，但从不作为结构化状态使用。

## 错误

```text
GitlancerError
  - Domain(DomainError)   NotARepository, NotAWorktree, PathOutsideWorktree, WorktreeMismatch,
                          MainWorktreeDeletionUnsupported, BranchNotFound, BranchAlreadyExists
  - Exec(GitExecError)    GitNotFound, SpawnFailed, NonZeroExit, OutputTooLarge, OutputReadFailed
  - Parse(ParseError)     MissingLine, InvalidWorktreeList, InvalidStatus, InvalidCheckpoint
  - DiffTooLarge, CommitMetadataUnavailable, Io
```

## 测试

解析器和领域校验有单元测试，命令构造用 `RecordingGitRunner` 测试，真实仓库的集成测试覆盖多 worktree 场景：嵌套目录发现、main 和链接 worktree 列表、基础解析、在显式 commit 上创建、从链接 worktree 提交、不匹配检测、porcelain v2 状态，以及链接 worktree 的 `.git` 间接引用。
