# 任务与 worktree

[English](tasks-and-worktrees.md) | 中文

每个项目有一个 **Main** Workspace，也就是项目自己的 checkout。**任务**是一个 **Isolated** Workspace 面向用户的标签，背后是一个链接的 Git worktree。会话和工作流运行都在某个 Workspace 中执行；worktree 本身是 backend 拥有的状态，没有任何契约携带 worktree id 或提供 worktree 操作。

## Workspace 归属

公开的 `Task` 载荷暴露 `workspaceId`。`worktrees.workspace_id` 既是主键也是 Workspace 外键，所以 Git 元数据是 Workspace 可选的一对一扩展，而不是第二个身份。`UpdateTaskRequest` 不能修改项目或 Workspace 归属，`CreateTaskRequest.baseBranch` 是必填的。

## 创建任务

`CreateTaskHandler` 在 `TaskWorktreeProvisioner` 端口之后完成整个流程，因此没有任何 Git 类型进入契约，单元测试也可以换成假的 provisioner：

1. 检查项目根目录是 Git 仓库；不是时 worktree 模式显式失败。Desktop 会依据 `listWorkspaces` 报告的 `git` 能力，事先为这类项目隐藏新建 worktree 的入口，见 [Workspace 评审](workspace-review.zh.md#没有-git-的目录)。
2. 把选中的本地基础 ref 解析为不可变的 commit id。ref 已经不存在时，在创建任何东西之前返回 `base_branch_not_found`。
3. 预留 Task 和 Workspace id，保证它们的短分支前缀不冲突（见下文）。
4. 持久化一个 **provisioning 租约**，写明精确的仓库根、checkout 路径和分支。
5. 在解析出的 commit 上创建分支和链接 worktree，Git 运行期间持续续租。
6. 在一个事务（`SqliteTaskWorkspaceRepository`）里提交 Workspace、provisioning 结果、`Worktree` 行和 `Task`；这个事务还会重新确认项目仍然可见，并删除租约。

分支名是 `ora/<Workspace id 的前 8 个字符>`，checkout 目录是 `<worktree 根目录>/<完整 Workspace id>`。因为分支名被截短了，只要已有任务 worktree 目录以该前缀开头，或者本地已有 `ora/<prefix>` 分支，生成的 id 就会被拒绝；所以一个 checkout 已被删除的孤立分支仍会占住它的前缀。尝试三次后 handler 直接失败，不会无限循环。

在任何时刻，已创建的 Git 资源都只有一个持久的所有者：租约或已提交的行。如果 Git 创建资源之后出了问题（持久化错误、项目被并发删除并返回 `project_not_found`，或进程崩溃），handler 会把租约交给持久化清理，或者租约因没有续期而过期，由清理 worker 接手。部分创建的 Workspace 永远不会暴露出来，Desktop 报告的是不含原始 Git 输出和路径的类型化错误。

## 基础分支

分支列表只读取本地 `refs/heads`。打开选择器和创建 worktree 都不会 fetch、prune 或改动远程跟踪 ref，所以创建可以离线进行，使用的是仓库里已有的分支状态。`ListProjectBranchesHandler` 把这些 ref 与项目的任务和 worktree 记录合并：`ora/<prefix>` 分支保留精确的 `refName`，但显示所属任务的标题。创建 worktree 会让该项目的分支列表失效，新分支可以立即作为下一个任务的基础。

## 解析 checkout

配置的 worktree 根目录只是创建目标。修改它只影响之后开始的创建；进行中的操作使用原来的快照，已有的 worktree 从不移动。已有路径从不根据根目录重新拼接：任务的 checkout 总是实时解析，路径为 Task → Workspace → 存储的分支名 → `git worktree list --porcelain`。Agent 会话、Workspace 评审、`resolve_task_cwd` 和 `resolve_workspace_cwd` 都使用这条解析路径。Main Workspace 的存储位置按 Desktop 的路径基准解析，见 [Desktop 宿主](../desktop/desktop-host.zh.md#持久化路径)。

## 审阅大型变更

Git 比较正在加载或失败时，仍然可以在「变更」和「文件」之间切换。完整 Diff 超过预览上限时，
「变更」会列出当前 Git 范围内的变更文件，并按选择加载单个文件 Diff 及变更附近的上下文。
单文件仍超限时，该文件保留在清单中，其他文件可以继续审阅。

## 删除与持久化的 Git 清理

任务、项目和工作流运行的删除遵循同一条规则：**数据库级联即成功，物理 Git 清理是持久且异步的**。`SqliteCascadeRepository` 在一个事务里软删除整个聚合（会话和所属的 worktree 记录），只要还有下级会话处于 `Running`，就以 `resource_in_use` 拒绝。在同一个事务里，它读取每个有 worktree 的任务的仓库根、分支名和记录的 checkout 路径，并为每个任务插入一行 `git_cleanup_jobs`，所以提交之后的崩溃、断电或 SIGKILL 都不会丢失清理目标。

Backend 的清理 worker（`crates/backend/src/git_cleanup`）处理待办任务和过期的 provisioning 租约：

- 它强制删除链接的 worktree（按分支解析；detached 的 worktree 按记录的路径解析），并强制删除本地 `ora/<prefix>` 分支。两个阶段都是幂等的：资源已经不存在就是确认成功。
- 删除以磁盘上的结果为准。当 Git 注销了 worktree 却留下了目录（Windows 上常见的半失败），空的残留目录直接删除，有内容的残留则作为可重试的失败。对于已经无法证明归 Ora 所有的非空 checkout，worker 不做任何改动，任务停在 `manual_attention`。
- 任务没有持久化的"运行中"状态，所以在任何时刻崩溃都可以重放；启动后的第一轮就是协调，另有每 60 秒一次的扫描找回丢失的唤醒。
- 一批最多取 16 个任务，按仓库分组；最多四个仓库并发，同一仓库内的任务串行执行。带退避的五次失败之后，任务停在 `manual_attention`。
- 短时间读取 checkout 的操作（diff、提交、推送）持有共享的 worktree 使用租约，清理时以独占方式获取；同一仓库的所有 worktree 和分支变更都由仓库闸门串行化。加锁顺序固定：先租约，后闸门。

远程分支从不被触碰，provider 拥有的 ACP 历史也从不被删除。清理结果以 `operation = "git_cleanup"` 记录日志，与删除任务的那次请求无关。
