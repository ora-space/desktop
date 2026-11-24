# Workspace 评审

[English](workspace-review.md) | 中文

Changes 面板评审任意 Workspace checkout（任务 worktree 或项目的 main checkout）中的 Git 改动，并负责暂存、提交和推送。用例在 `crates/application/src/workspace_diff`，UI 在 `packages/app-shell/src/features/diff`。

## 操作

| 操作                                               | 用途                                   |
| -------------------------------------------------- | -------------------------------------- |
| `getWorkspaceDiff`                                 | 某个范围的有上限的 unified patch       |
| `getWorkspaceStatus`                               | 按文件的暂存状态                       |
| `stageWorkspaceChanges`、`unstageWorkspaceChanges` | 暂存或取消暂存路径；空列表表示全部改动 |
| `commitWorkspaceChanges`                           | 精确地提交已暂存的改动                 |
| `pushWorkspaceBranch`                              | 把当前分支推送到 `origin`              |

请求携带 `WorkspaceId`；backend 解析当前的 checkout，从不接受前端给的路径或 `TaskId`。

## 没有 Git 的目录

项目可以是任意目录；这时 Agent 直接写入该目录，没有可评审的改动。`listWorkspaces` 把存在的目录报告为
`{ kind: "present", git }`。`git` 来自对每个目录执行一次 `git rev-parse --is-inside-work-tree`，每次列出都重新执行，从不存储：

| `git`           | 含义                                                                                                  | 客户端行为                                                                                                       |
| --------------- | ----------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `repository`    | 该目录位于某个工作树内                                                                                | 不变                                                                                                             |
| `notRepository` | Git 明确表示该目录不在任何仓库中，且该目录及其上级都没有 `.git`                                       | Changes 显示说明，不发起 diff 或状态查询；隐藏所有新建 worktree 的入口；定时任务表单说明为何无法为该项目创建任务 |
| `unknown`       | Git 无法回答（未安装、所有者不受信任、`.git` 损坏或不可读、5 秒内没有结果，或该 checkout 正在被清理） | 不变，让操作自身的错误照常显示                                                                                   |

Git 会向上查找仓库，所以嵌套在另一个 checkout 里的普通目录算作 `repository`，评审操作作用于外层 checkout。运行
`git init`（或删除 `.git`）后，客户端在下一次重新拉取 workspace 列表时看到变化：窗口重新获得焦点且缓存的列表已超过 30 秒的过期时间、Changes
面板里的“重新检查”按钮，或者一次 `workspace_not_git_repository` 拒绝（Changes 面板会因此立即重新探测）。在这种目录里，
本应打开 diff 的聊天链接改为在 Files 中打开该文件。

上表中的每个操作都会先用同样的方式探测解析出的 checkout；目录不在任何仓库中时，在基线检查之前、在运行操作自身的
Git 命令之前返回 `workspace_not_git_repository`。

## 范围与基线

`WorkspaceDiffScope` 取值为 `Branch`、`Unstaged`、`Staged` 或 `Committed`。

- 有 `Worktree` 行的 Workspace（任务）有固定的基线，即创建 worktree 时所基于的 commit。`Branch` 和 `Committed` 与它比较。提交或推送之前，handler 会把 checkout 的路径和分支与记录的状态核对。
- 没有 `Worktree` 行的 Workspace（项目的 main checkout）没有基线，只能用 `Unstaged` 和 `Staged`，写操作直接作用于当前 checkout 出来的内容，不做核对。
- 没有基线时请求 `Branch` 或 `Committed`，会在运行 Git 之前返回 `workspace_diff_baseline_unavailable`。

patch 在进入内存之前就有上限：过大的 patch 返回 `workspace_diff_too_large`，被丢弃的大小不会进入契约。空的提交信息返回 `workspace_diff_commit_message_blank`。Git 和持久化故障保留错误链，并投影为 `internal_error`。推送使用 `GitIntent::Network` 并禁用交互提示，见 [Gitlancer](gitlancer.zh.md)。

短时间的读取和写入持有共享的 worktree 使用租约，所以 diff 或推送进行期间提交的删除，不会把 checkout 从它们脚下删掉，见[任务与 worktree](tasks-and-worktrees.zh.md#删除与持久化的-git-清理)。

## 刷新

`state/data/workspace-review.ts` 拥有 `refreshWorkspaceReview(queryClient, workspaceId)`。它让该 Workspace 的所有 diff 范围和暂存状态失效；活跃查询并发地重新拉取，不活跃的标记为过期，其他 Workspace 的缓存不受影响。

- 暂存和提交在操作内部等待这次刷新，即使写入进行期间视图切换了，也仍然针对发起时的 Workspace。
- 手动刷新、prompt 完成后的清理，以及防抖后的 Agent 文件变化或回合结束事件都使用同一个接口；消费事件从不等待网络响应。
- 每次刷新都会先取消更早的 diff 和状态请求（包括首次加载），再发起新请求，所以迟到的响应不会覆盖较新的快照。重叠的刷新在同一个 `QueryClient` 和 Workspace 上共享一次完成，直到最新一轮结束（包括重试和离线暂停）；被替代的轮次不能让这些等待者结束。
- 刷新失败仍是查询错误，从不把一次成功的 Git 写入变成失败的变更；视图显示错误并提供重试。

## 渲染大量改动

面板把 backend 的 patch 拆成按文件的片段，并缓存每个解析后的文件，所以只改了少数文件的刷新会保留其余文件的 memo 渲染。超过大约 512 KiB 字符的 patch 在首次绘制之后再解析，期间显示加载状态。

显示方式由 diff 的规模决定，没有手动切换。小 diff 使用一个连续滚动的主体。大的 diff（文件多、改动行多，或者某个文件大到需要分窗口）切换为由文件树驱动的单文件聚焦主体。在聚焦主体中，改动超过 400 行的文件被切成固定大小的原生 `react-diff-view` hunk，只用 `@tanstack/react-virtual` 挂载可见的窗口，所以聊天引用跳转、按行引用和折叠展开在真实的行上照常工作。导航细节见 [`features/diff/README.md`](../../packages/app-shell/src/features/diff/README.md)。
