# 工作流

[English](workflow.md) | 中文

`ora-application` 负责工作流定义用例，`ora-db` 负责持久化，`ora-contracts` 定义公共契约。工作流管理可编辑的 Agent 编排图，以草稿作为编辑工作区，以不可变发布快照作为运行版本。

## 实体与数据表

| 领域类型           | 数据表               |
| ------------------ | -------------------- |
| `Workflow`         | `workflows`          |
| `WorkflowSnapshot` | `workflow_snapshots` |

`Workflow` 保存稳定身份、名称、发布快照指针和审计字段；`WorkflowSnapshot` 保存版本化的 React Flow 图。详情、摘要和版本读模型分离，列表响应不包含图数据。列表按创建时间倒序排列。

## 草稿、发布与版本生命周期

创建工作流时原子创建唯一 `draft` 快照。`UpdateDraft` 就地更新草稿图，不生成新快照。

复制工作流时，从源工作流的当前草稿创建新的身份和草稿，不复制发布快照、当前发布指针或已有运行。副本从未发布状态开始。

发布将草稿复制为新的不可变快照，并更新 `workflows.published_snapshot_id`，供后续运行使用。发布快照的 `updated_at` 始终为 `NULL`，包括软删除之后；只有草稿编辑会更新该字段。

- 回滚：把历史快照复制到草稿，不改变发布指针。
- 激活：切换发布指针，并将该快照同步到草稿。
- 删除快照：可删除单个发布快照，但不能删除草稿或当前激活版本。

## 标识符与版本号

`WorkflowId` 和 `WorkflowSnapshotId` 使用基于 UUID 的新类型，遵循仓库的 `define_id!` 约定。

版本号是字符串，`draft` 为保留值。发布版本可由用户指定，也可通过注入时钟生成 `v{timestamp_millis}`；同毫秒冲突时追加数字后缀。用户版本不能为空，不能超过 128 字节，必须适合作为单个 URL 路径片段，且不能为 `.` 或 `..`。部分唯一索引约束同一工作流的可见版本，已软删除的版本号可以重用。

## 图存储

`graph` 字段保存完整 React Flow JSON。工作流定义 CRUD 将其视为不透明字符串；[工作流运行引擎](../crates/application/src/workflow_run/engine/README.md)在启动时解析和校验冻结快照。

## Agent 节点 MCP 绑定

节点配置读取已安装的 `kind: "mcp"` 插件，展示名称、完整 ID 和配置可用性。每个节点独立添加、启用、禁用和移除绑定，添加后默认启用。加载失败时提供重试并保留现有配置；插件缺失时仍显示 ID，允许禁用或移除。

图中使用 `mcps: [{ mcpId, enabled }]` 保存绑定，其中 `mcpId` 是完整插件 ID。草稿保存、发布、复制和导入导出均保留绑定，包括已禁用项。空列表或旧图缺少 `mcps` 时表示节点不使用 MCP。旧示例 ID 原样保留，不猜测替代插件。执行图解析会拒绝空 ID、重复 ID 和非法字段类型。

会话创建、恢复、重建和刷新始终使用冻结运行中的已启用 ID。草稿修改只影响后续运行。已启用的依赖不可用时明确报告节点失败，不静默跳过；未选择的插件不阻塞节点。普通聊天仍自动发现可用插件。选中插件的版本和配置更新继续使用现有安全刷新边界，图中不保存凭据。详见 [Session MCP](session-mcp.zh.md)。

## 处理器

处理器采用端口与适配器模式，共用 `WorkflowRepository`、`WorkflowIdGenerator` 和 `Clock`。

| 处理器                    | 用途                   |
| ------------------------- | ---------------------- |
| `CreateWorkflowHandler`   | 创建工作流与草稿       |
| `GetWorkflowHandler`      | 获取完整详情           |
| `ListWorkflowsHandler`    | 列出摘要               |
| `UpdateWorkflowHandler`   | 更新名称               |
| `DeleteWorkflowHandler`   | 软删除工作流与快照     |
| `UpdateDraftHandler`      | 更新草稿图             |
| `PublishWorkflowHandler`  | 发布并激活新快照       |
| `RollbackWorkflowHandler` | 将历史快照复制到草稿   |
| `ActivateWorkflowHandler` | 激活快照并同步草稿     |
| `ListVersionsHandler`     | 列出发布版本           |
| `GetVersionHandler`       | 按版本获取快照         |
| `DeleteSnapshotHandler`   | 在约束范围内软删除快照 |

工作流定义删除使用普通 CRUD 处理器，不采用项目和任务的独立级联仓库；运行引用的保护规则另见下文。

## 工作流运行

运行通过 `workspace_id` 绑定现有工作区。Agent 节点在选定工作区执行，每个节点创建独立会话，冻结的执行配置决定 Agent、模型、角色、Skill、MCP 和提示词。

Skill 在部署时校验并物化，运行保存包含调用名称和包路径的物化回执。调用名称和落点来自该回执，执行节点不再按名称重新读取可变的全局目录。Skill 交付通过 `AgentSkillDeliveryProvider` 隔离，发现根目录的数量和位置可随 Agent 实现变化，不必改变工作流创建或提示词组装逻辑。

节点完整对话以会话历史为唯一来源。`workflow_node_runs.output` 保存最终助手文本，用于展示、审计及 `agent-1.output` 访问；即使结构化解析或校验失败也保留原文。成功校验的结构化对象才写入 `agent-1.structured_output`。

运行变量池保存在 `workflow_runs.payload`。它包含有类型的 Start 输入和产出数据节点的稳定输出，`variablePool.values` 只保存已赋值变量。Condition 不公开输出变量，其分支选择单独保存在 `conditionDecisions`，能够跨重启恢复。完成节点的变量写入与状态切换在同一 SQLite 事务中提交。

运行的启动指令保存在 `workflow_runs.input`，不会作为普通工作流变量供选择。Start 输入可以在定义中暂不赋值，部署前填写。自定义全局变量必须包含显式点分名称与类型正确的初始值；系统全局变量由运行时拥有。每个节点可使用全局变量与直接前驱变量，Condition 对变量作用域透明，连续 Condition 会传递原始前驱变量。其他间接祖先变量必须通过直接前驱转发。结构化字段路径进入同一变量目录，Condition 和 Output 无需手写自由文本选择器。各终端 Output 独立构建结果对象，结果名只需在同一 Output 内唯一。

变量类型在图解析、编辑器输入和变量写入时检查。`array` 与 `array[any]` 允许异构数组；有类型数组检查每个元素。文件使用工作区相对引用 `{ "kind": "workspace_file", "path": "relative/path" }`，文件数组使用该对象数组。旧路径字符串会被规范化；绝对路径、父目录穿越、空路径和平台保留路径被拒绝。结构化 Agent Schema 递归校验，并在提示词中生成有效示例。Agent 输出的文件字段必须遵循该对象形式。

Start 表单控件与变量类型分离：文本、段落、选择框、数字、复选框、单文件、文件列表和 JSON 分别产出 `string`、`string`、`string`、`number`、`boolean`、`file`、`array[file]`、`object`。展示名称、选项、必填项和文本长度限制随快照冻结，并在部署边界重新校验。旧快照按变量类型推导兼容控件。

### 实体与状态

| 领域类型          | 数据表               |
| ----------------- | -------------------- |
| `WorkflowRun`     | `workflow_runs`      |
| `WorkflowNodeRun` | `workflow_node_runs` |

`WorkflowRun` 固定引用发布版本的 `snapshot_id`，保存运行名称与工作区。`WorkflowNodeRun` 只为实际开始的节点创建记录，未开始状态由前端对比图与记录推导。

运行与节点均使用 `Pending | Running | Succeeded | Failed | Cancelled`。交互节点等待后续输入时持久化为 `Pending`；公共契约将存在等待节点的运行投影为 `AwaitingInput`。终态节点会话只读，后端拒绝新提示词。节点会话可按 ID 读取，但不会出现在普通聊天列表中。

### 创建与快照固定

创建处理器验证指定工作区和快照归属，使用显式 `snapshot_id` 或工作流当前发布版本，并校验角色与 Skill 绑定。运行初始为 `Pending`，`current_nodes` 为空。变量池从冻结图编译，并以启动文本初始化保留的 `{start_id}.input`。未提供启动文本时，使用冻结 Start 指令作为默认值。

部署界面收集运行名称、可编辑的启动指令和 Start 输入。工作区由打开工作流选择器的项目或任务明确提供，后端不推断分支，也不额外创建 worktree。

### 读取、删除与保护

详情返回运行与节点，列表按项目提供摘要。此层只读取节点历史，节点写入和状态机由引擎负责。

删除会拒绝活动运行、等待人工输入且含非终态节点的运行，以及拥有运行中节点会话的运行。尚未启动且没有节点记录的 `Pending` 运行可以直接删除。删除会软删除运行、节点及其会话，绝不删除共享工作区。软删除记录不可重新激活。

活动运行引用的发布快照不能软删除（`SnapshotInUse`），其工作流也不能删除（`ActiveRuns`），确保冻结图始终可读取。

## 职责边界

- 运行 CRUD 负责持久化，`ora-application` 引擎负责调度、节点写入和状态机，后端 `WorkflowRunNodeExecutor` 驱动会话。
- 变量池使用已有 JSON 字段，不引入新的变量表；MCP 选择使用已有图和会话关联，不增加文件布局或数据库迁移。
- 执行图验证属于运行引擎，不属于定义 CRUD。
- Tauri 命令和服务器路由属于传输适配器。

另见[领域模型](domain-models.md)、[应用与契约边界](application-contracts-boundary.md)、[数据库仓库](database-repositories.md)、[运行引擎](../crates/application/src/workflow_run/engine/README.md)和[后端](../crates/backend/README.md)。
