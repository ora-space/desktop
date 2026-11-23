# Worktree 执行

[English](worktree-execution.md) | 中文

除了 clone，Node 还可以在显式绑定给它的已有 Main Workspace 下创建和删除任务 worktree（`ensure_worktree`、
`remove_worktree`，能力为 `worktree_execution`）。这是 Node 的第一个执行族；它的
[决策](https://github.com/ora-space/specs/blob/main/decisions/node/worktree/0-worktree-management.md)已被取代，Cloud
路径改用 [clone](repository-clone.zh.md)，但这个执行族仍然实现着，并遵循同样的持久性规则。Desktop 自己的任务 worktree
是另一条进程内路径，见[任务与 worktree](../workspaces/tasks-and-worktrees.zh.md)。

## 绑定与目标

`node.repositories` 列出 `RepositoryBinding`：仓库引用、已有 Main Workspace 的身份和路径、授权根目录和受管的
worktree 根目录。命令必须匹配某个绑定；Node 用 Git 校验主 checkout，并要求 Git 中的主仓库登记与绑定一致（只是能在
目录里成功运行 Git 是不够的；链接的元数据通过它的 `commondir` 和 `gitdir` 反向引用来检查）。

`NodeManaged.directory_name` 必须是一个可移植的目录段。路径用 `Path`/`PathBuf` 和共享的规范化包含检查构建；静态的
符号链接和与主 checkout 的重叠会被拒绝。静态检查无法防御在校验和使用之间恶意替换符号链接。

受理时冻结目标：绑定路径、Git 元数据目录、授权根目录和受管根目录、任务路径、分支和不可变的基础 commit。重传从不重新
解析已经移动的基础 ref，也不替换已保存的路径，每次变更和恢复都会重新检查这些事实。

## 执行

在 Git 改变任何东西之前，输入已经持久化，Git 也从不在 SQLite 事务中运行。顺序是：校验并去重，解析，持久受理并占用
资源（Workspace、路径、分支），持久化变更意图，把 Git 作为已记录的尝试运行（见 [Node](node.zh.md#执行与恢复)），检查实际的事实，然后与原始事件一起
原子地完成。

- 创建在冻结的路径、分支和 commit 上运行，并校验得到的 checkout；含糊的部分效果留给恢复处理。
- 删除只强制删除自己拥有的链接 checkout 和任务分支，分为可以独立恢复的几个阶段，只有确认登记、目录和本地分支都已经
  不在（只删除自己拥有的空残留目录）之后才报告成功。`AlreadyAbsent` 要求每个目标都不存在。主 worktree、其他分支和不属于
  自己的非空目录从不被强制清理。
- 前置条件失败会提交结构化的失败和事件；格式错误的消息和身份冲突在入口就被拒绝。确定没有产生任何效果的创建失败会
  撤销它的占用，同时保留输入和结果。

## 恢复

| 证据 | 状态 | 下一步 |
|---|---|---|
| 输入已保存，Git 未启动 | `Accepted` | 变更之前先持久化运行意图 |
| 变更可能已经发生 | `Running` | 继续，或在重启后对账 |
| 证据不足 | `Unknown` | 再次观察；只有事实充分时才完成 |
| 结果和事件已提交 | `Completed` | 返回原来的结果并重放事件 |

恢复保留原来的身份和目标，从不选择新的路径、分支或基础：

- 与登记、路径和分支都匹配的、有效的自有 checkout 会完成创建，即使上面有新的 commit，也保留原来的基础。
- checkout 已删除但自有分支还在时，继续清理分支。
- 只剩停在冻结基础上的自有分支、没有冲突的 checkout 也没有无法解释的文件时，在持久化的 `CleanupCreation` 意图下清理，
  每一轮用原来的执行重试一次。
- 不一致的登记、改变的绑定或失败的检查保持 `Unknown` 并附带诊断，从不变成终止的 `ResultUnknown`。在已撤销的归属下
  重新出现的资源会让新的清理操作以 `WorktreeConflict` 失败，且不碰它们。
- `Unknown` 只阻塞冲突的资源；无关的工作仍可受理。

同一个执行下改变了操作类型或输入，或者把一个操作重新绑到另一个执行，会返回 `IdentityConflict` 并保留旧数据。
`RequestId` 只用于关联。

## 接口

`Node::open(NodeConfig, ProcessConfig, Shutdown)` 组装受管的 Git 和 Ora 的本地时钟；测试使用
`Node::open_with_dependencies` 注入 Git、写入守卫和时钟。`submit(Command::Ensure | Remove)` 返回 `ExecutionStatus`，
`status` 读取持久的证据，`pending_events` 和 `acknowledge` 处理投递，`state` 报告 `Ready` 或 `RecoveryPending`，
`recover` 对未完成的记录对账。变更操作接受 `&mut Node`，所以调用方把它放在互斥锁后面共享。
