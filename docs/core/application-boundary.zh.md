# 应用边界与领域模型

[English](application-boundary.md) | 中文

Desktop 的持久化操作分属六个 owner，因此用例编排、存储和 ACP 流都独立于 UI 外壳。本文说明各自负责什么、公开契约的形状、handler 集合，以及调用方可以依赖的不变量。

## 职责划分

| Owner          | Crate                    | 负责                                                                                                                                          |
| -------------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| 领域           | `ora-domain`             | 有数据表支撑的实体、标识符 newtype、分类枚举，以及与传输和存储无关、始终成立的不变量                                                          |
| 契约           | `ora-contracts`          | 请求、响应、流事件和公开错误 DTO；导出 TypeScript 到 `@ora/contracts`                                                                         |
| 应用           | `ora-application`        | 用例 handler、`ApplicationError`、handler 依赖的仓储/时钟/身份/provisioning 端口、领域到契约的映射                                            |
| 持久化         | `ora-db`                 | 这些端口的 SQLite 实现和 schema 协调，见[持久化](persistence.zh.md)                                                                           |
| Backend        | `ora-backend`            | 组装：SQLite 启动、系统时钟、具体仓储与 handler、agent 运行时宿主、插件生命周期、工作流执行、持久 Git 清理、公开错误投影和共享请求生命周期    |
| Desktop 适配层 | `apps/desktop/src-tauri` | 很薄的 Tauri 命令：接收契约请求，调用 Backend 领域句柄或 Desktop 文件系统服务，再序列化结果，见 [Desktop 宿主](../desktop/desktop-host.zh.md) |

- `ora-contracts` 的 Rust 字段名保持惯用写法，JSON 统一序列化为 `camelCase`。
- ACP v1 线上类型归官方的 `agent-client-protocol-schema`（Rust）和 `@agentclientprotocol/sdk`（TypeScript）所有。契约可以内嵌这些类型，但绝不复制一份 ACP schema。
- 前端端点目录（操作名、客户端命名空间、DTO 名、unary 或 stream 模式）归 `xtask`，见[前端契约](frontend-contracts.zh.md)。
- 通用文件系统浏览刻意放在 `ora-backend` 之外：Desktop 在命令层组装有界的工作区文件服务，见 [Workspace 文件](../workspaces/workspace-files.zh.md)。

## 领域模型

`ora-domain` 不依赖 SQLite、Tauri 或 `ora-contracts`，只依赖它就能构造和检查所有模型。

| 领域类型                                                   | 数据表                                                             | 说明                                                                                            |
| ---------------------------------------------------------- | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| `Project`                                                  | `projects`                                                         | 名称和仓库信息；它的 checkout 是 Main Workspace 的位置                                          |
| `Workspace`、`WorkspaceProvisioning`                       | `workspaces`、`workspace_locations`、`workspace_provisioning`      | 会话和工作流运行共享的稳定执行环境；`Main`（项目自身的 checkout）或 `Isolated`（任务 worktree） |
| `Task`                                                     | `tasks`                                                            | Isolated Workspace 面向用户的标签，本身不带状态                                                 |
| `Worktree`                                                 | `worktrees`                                                        | Workspace 可选的一对一 Git 扩展，以 Workspace id 为键                                           |
| `Session`                                                  | `sessions`                                                         | Workspace 中的一个对话，以及它当前所在的 provider session                                       |
| `Skill`                                                    | `skills`                                                           | 带命名空间的 Skill 目录条目                                                                     |
| `AgentDefinition`                                          | `agents`                                                           | 可配置的 Agent：带命名空间的名称、描述和 Markdown 内容                                          |
| `Workflow`、`WorkflowSnapshot`                             | `workflows`、`workflow_snapshots`                                  | 可编辑草稿加不可变的发布快照，见[工作流](../workflows/workflows.zh.md)                          |
| `WorkflowRun`、`WorkflowNodeRun`、`WorkflowExecutionScope` | `workflow_runs`、`workflow_node_runs`、`workflow_execution_scopes` | 已执行的运行，见[工作流运行](../workflows/workflow-runs.zh.md)                                  |
| `ScheduledTask`、`ScheduledRun`                            | `scheduled_tasks`、`scheduled_runs`、`scheduled_occurrences`       | 见[定时任务](../scheduled-tasks/scheduled-tasks.zh.md)                                          |
| `GitCleanupJob`                                            | `git_cleanup_jobs`                                                 | 持久化的物理 Git 清理，见[任务与 worktree](../workspaces/tasks-and-worktrees.zh.md)             |
| `WorktreeProvisioningLease`                                | `worktree_provisioning_leases`                                     | 创建任务期间 Git 资源的持有者                                                                   |

Workspace 的状态用相互独立的枚举表达，而不是一堆可选字段：`WorkspaceKind`（`Main`、`Isolated`）、`WorkspaceLifecycle`（`Provisioning`、`Active`、`Unavailable`、`Retiring`、`Deleted`）、`WorkspaceLocation`（本地文件系统、SSH，或只保存引用和不透明定位符、从不保存凭据的远程目标）以及 `WorkspaceProvisioningState`。

### 标识符

标识符是由同一个宏生成的 newtype，所以不能把 `TaskId` 传给需要 `ProjectId` 的地方：`ProjectId`、`WorkspaceId`、`TaskId`、`ScheduledTaskId`、`ScheduledRunId`、`GitCleanupJobId`、`WorktreeProvisioningLeaseId`、`SessionId`、`SkillId`、`AgentDefinitionId`、`WorkflowId`、`WorkflowSnapshotId`、`WorkflowRunId`、`WorkflowScopeId` 和 `WorkflowNodeRunId`。每个都包装一个 `String`，序列化为裸字符串。`Worktree` 以所属的 `WorkspaceId` 作为身份。

### 分类字段

封闭的类别一律用枚举表示，不用原始编码，例如 `SessionStatus`（`Running`、`Stopped`）和 `WorktreeActivity`（`Inactive`、`Active`）。每个枚举都提供 `database_value()`，以及会以 `DomainModelError` 拒绝未知持久化值的 `from_database_value()`；不存在从整数的隐式转换。

Agent 身份刻意不是封闭枚举。`AgentRef` 是对提供该 Agent 的插件包规范 id 的校验 newtype（例如 `official/ora-space.claude`），因为每个 Agent 都来自已安装的插件，构建时无法知道有哪些。`AgentRef::parse` 只拒绝空值；无法识别的身份表示"当前未安装"，而不是数据损坏。

### 领域层校验

大多数模型是构造不会失败的普通快照，例外如下：

- `Skill::new` 去掉名称首尾空白并拒绝空名称。名称必须是单个文件系统安全的片段（ASCII 字母数字加 `.`、`_`、`-`，最多 255 字节，不能以 `.` 开头），这样已提交的 Skill 永远不会落到 Skill 根目录下的保留目录或隐藏目录上。
- `AgentDefinition::new` 去掉名称首尾空白并拒绝空名称。
- `SessionTitle::parse` 去掉 Agent 提供文本的首尾空白，拒绝空值，超过 255 个 Unicode 标量值时拒绝而不截断。`Session.title` 是 `Option<SessionTitle>`。

`Session.workspace_id` 在构造时固定，因为它决定工作目录。`agent_ref` 和 `agent_session_id` 是当前绑定而不是身份：对话换到另一个 Agent 时，`with_binding` 同时替换两者，id 和已记录的历史保持不变。provider 返回 session id 之前，`Session` 不可能存在。`HistoryState` 是另一个独立维度，持久化为一个可空的原因列：为空表示 `Writable`，非空表示 `Degraded`，并附上用户需要处理的原因。`SessionMcpSelection` 是 `Automatic` 或一组显式的插件 id，见 [Session MCP](../agents/session-mcp.zh.md)。

`AuditFields` 为每个可软删除的实体携带 `created_at`、`updated_at`（Unix 毫秒）和 `is_deleted`。只有可空的列才对应可选字段。

## 契约形状

契约是面向应用的协议，不是领域模型的投影。每个实体只有一个公开视图模型，在创建、获取、列表和更新响应中复用，例如：

- `Project`：`id`、`name`；`CreateProjectRequest` 接收 `name` 和 `mainWorkspacePath`
- `Task`：`id`、`projectId`、`workspaceId`、`title`
- `Session`：`id`、`workspaceId`、`title`、`agentRef`、`status`、`historyState`、`mcpSelection`
- `Skill`：`id`、`namespace`、`name`、`description`、`source`（本地或插件）、`availability`
- `Agent`：`id`、`namespace`、`name`、`description`
- `ProjectBranch`：`name`、`refName`、`displayName`

公开载荷只包含有文档的业务字段，`isDeleted` 等审计字段从不出现。工作流摘要和快照的时间戳是刻意的例外，因为版本历史和编辑器新鲜度对用户可见。

- 任务 worktree 归 backend 所有：没有契约携带 worktree id，也没有 worktree 操作。`CreateTaskRequest` 接收 `projectId`、`title` 和 `baseBranch`。
- 会话私有的 provider session id 会持久化并用于 `session/load`，但从不暴露。
- Workspace 文件契约携带任务或项目身份和规范化的相对路径；由服务端解析根目录，调用方从不提供根目录。
- Workspace diff 契约暴露 patch 快照（`baseCommitId`、`headCommitId`、`patch`），既不带文件系统路径，也不带 Git 诊断信息。

## 公开错误

公开失败直接序列化为 `{ code, params, requestId }`。`RequestId` 是 UUID，`PublicError` 是判别联合，每个 code 对应一种参数形状（包括显式的空参数）。没有内部消息，也没有外层信封。

Backend 把最高层的语义 `ApplicationError` 穷尽地映射为一个公开错误和一个与传输无关的分类：`InvalidRequest`、`PayloadTooLarge`、`NotFound`、`Conflict`、`Unprocessable`、`Forbidden`、`HostUnavailable` 或 `Internal`。基础设施故障变成 `internal_error`，其 `Error::source()` 链仍保留给日志使用。适配层从不通过检查错误链或匹配错误字符串来推断公开 code。各分类的日志级别见[日志](logging.zh.md)。

## Handler

每个 `ora-application` handler 只接收一种请求契约，只返回一种响应契约或 `ApplicationError`，不引用任何 UI、Tauri 或数据库类型。依赖通过泛型静态分发，因此单元测试可以用内存假实现构造任意 handler。

| 模块               | 用例                                                                                |
| ------------------ | ----------------------------------------------------------------------------------- |
| `project`          | 创建、获取、列表、列出分支、更新                                                    |
| `task`             | 创建、获取、列表、更新                                                              |
| `session`          | 获取、列表、删除                                                                    |
| `skill`            | 创建、获取、列表、更新、删除、启动时协调                                            |
| `skill_import`     | 准备、获取、提交、取消                                                              |
| `agent_definition` | 创建、获取、列表、更新、删除；Markdown 导入（准备、提交）                           |
| `effect`           | Desired State 与 Target 状态，见 [Effect](../agents/effects.zh.md)                  |
| `workspace_diff`   | 读取 diff 和状态、暂存、取消暂存、提交、推送                                        |
| `workflow`         | 创建、获取、列表、更新、删除、分析、获取/更新草稿、发布、回滚、激活、版本、删除快照 |
| `workflow_run`     | CRUD，以及运行引擎（启动、取消、重启、恢复、完成节点）                              |
| `scheduled_task`   | 创建、获取、列表、列出历史、更新、删除、撤销授权                                    |
| `worktree`         | 没有 handler，只有端口                                                              |

这套 handler 集合意味着：

- Project 和 Task 没有删除 handler。聚合删除是 `ora-backend` 和 `ora-db` 负责的事务级联，因为它必须拒绝运行中的下级，并原子地更新多张表。
- 会话创建、prompt、权限、取消、切换 Agent 和历史记录都属于 agent 运行时，不属于 `ora-application`；session 模块只提供读取和软删除。见 [Agent 运行时](../agents/agent-runtime.zh.md)。
- `workspace_diff` 负责任意 checkout 的评审用例，但不负责选择 Workspace：由 backend 组装解析当前 cwd，以及存在 `Worktree` 行时它固定的基线。见 [Workspace 评审](../workspaces/workspace-review.zh.md)。
- `project_id`、`task_id` 和 `workspace_id` 是透传标识：创建和更新 handler 在委托给仓储之前不做额外的存在性检查。
- 尽管仓储实现的是软删除，删除在边界上仍是普通的删除用例。

### Agent 导入

`prepareAgentImport` 解析一个 Markdown 格式的 Agent 文件（最多 1 MiB，YAML front matter 含 `name` 和 `description`），并报告 `local` 命名空间里是否已有同名 Agent。`commitAgentImport` 重新解析内容并导入；遇到重名时，按调用方显式给出的 `skip` 或 `overwrite` 处理。调用方要传入准备阶段得到的冲突 Agent 的 id 和 `updatedAt`；如果存储的行在此期间变了，结果是 `stale_conflict`，不写入任何内容。

## 不变量

- 项目的 Main Workspace 位置在创建时确定；`UpdateProjectHandler` 只改名。
- 创建项目时，本地 Main Workspace 路径为空或不是已存在的目录会被拒绝，返回
  `project_workspace_not_directory`；相对路径按后续解析 cwd 时相同的基准目录解析。本地 Main Workspace
  从创建起就是 `active`：Ora 从不为本地 checkout 做 provisioning，因此目录是否仍然存在会在每次使用时
  重新检查并以 `workspace_unavailable` 报告，而不是固化进生命周期。
- 会话的 Workspace 不可变。生命周期操作只改 `status` 和 `updated_at`。
- `UpdateTaskRequest` 不能修改项目或 Workspace 归属。
- Project 和 Task 删除在一个 SQLite 事务里软删除 Ora 拥有的整个聚合。存在运行中的会话时以 `resource_in_use` 拒绝。这些路径从不调用 Git，也从不删除 provider 拥有的 ACP 历史，但会删除 Ora 自己记录的历史。
- 删除或创建失败之后的物理 Git 清理是持久、异步的，见[任务与 worktree](../workspaces/tasks-and-worktrees.zh.md)。

## 错误传播与完成日志

handler 和仓储保留语义错误与错误链，不发出通用的成功事件或只为传播而记录的失败事件。Tauri 命令和 channel 入口拥有唯一一条带关联 id 的请求完成事件，并按公开分类决定级别。启动、迁移、状态转换和清理事件是各自独立的生命周期事实。见[日志](logging.zh.md)。
