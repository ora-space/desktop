# 工作流

[English](workflows.md) | 中文

工作流是一张可编辑的 Agent 编排图，由一个可编辑的草稿和若干不可变的发布快照组成。定义归 `crates/application/src/workflow` 所有，由 `ora-db` 持久化，在 `packages/app-shell/src/features/workflow-editor` 中编辑。执行快照见[工作流运行](workflow-runs.zh.md)；Loop 和 Iteration 容器见[循环与迭代](loops-and-iterations.zh.md)。

## 定义与版本

`Workflow`（`workflows`）保存稳定的身份：名称、发布快照指针和审计字段。`WorkflowSnapshot`（`workflow_snapshots`）拥有带版本的 React Flow 图。读模型让列表不带图数据，工作流库按最新创建在前排序。

- 每个工作流恰好有一个草稿快照，与工作流一起创建。草稿是一个工作区：`updateDraft` 原地修改它的图，不新建行。
- **发布**把草稿复制成一个新的不可变快照，并总是激活它（`workflows.published_snapshot_id`），让它成为之后的运行使用的版本。发布快照的 `updated_at` 保持 `NULL`。
- **回滚**把某个历史快照的图复制进草稿，不改变发布指针。
- **激活**把发布指针切到另一个快照，并把它的图同步进草稿。
- **删除快照**删除一个发布快照，但拒绝删除草稿、当前激活的版本以及任何被活跃运行引用的快照（`SnapshotInUse`）。如果活跃运行冻结了某个工作流的快照，删除这个工作流会被拒绝（`ActiveRuns`）；否则删除会软删除工作流及其快照。
- **复制**从源工作流当前的草稿创建一个新的身份和草稿，不带发布快照、激活指针或运行。

版本是字符串。草稿使用保留的 `"draft"`。发布版本由用户提供（非空、最多 128 字节、单个 URL 路径片段、不能是 `.` 或 `..`），或由应用时钟派生为 `v{timestamp_millis}`，同一毫秒冲突时加数字后缀。一个部分唯一索引防止同一工作流出现重复的可见版本，所以已删除的版本名可以复用。

操作：`createWorkflow`、`getWorkflow`、`listWorkflows`、`updateWorkflow`（改名）、`deleteWorkflow`、`getDraft`、`updateDraft`、`publishWorkflow`、`rollbackWorkflow`、`activateWorkflow`、`listVersions`、`getVersion`、`getWorkflowSnapshot`、`deleteSnapshot` 和 `analyzeWorkflow`。文件通过原生命令 `write_workflow_export` 和 `read_workflow_import` 交换；打包的工作流以[工作流包](../plugins/workflow-packages.zh.md)的形式到来。

## 图存储

`graph` 列保存完整的 React Flow 文档。保存草稿时把它作为不透明字符串存下，所以未完成的画布也可以保存。发布时在从入口可达的子图上运行与创建运行相同的可执行图解析（见[运行引擎](../../crates/application/src/workflow_run/engine/README.md)）：可达节点的配置被解析器拒绝时，发布以 `workflow_run_graph_parse` 失败，不创建快照；不在这条路径上的备用节点可以保持不完整。创建运行时会再次解析冻结的快照。

### 在编辑器中加载

所有加载路径（草稿加载、版本预览、导出和运行视图）都通过 `@ora/workflow-runtime` 中同一个边界 `parseWorkflowGraphWithReport` 解析：

- 不是可用记录的节点，或 `data.kind` 不在 `WORKFLOW_NODE_KINDS` 中的节点，连同引用它们的边一起被丢弃。编辑器报告数量和种类（"已跳过 1 个本版本无法渲染的节点（router）"）；只读视图静默丢弃。
- 旧的 `prompt` 和 `model` 节点在检查种类之前升级为 `agent`。
- 丢弃只发生在读取时，从不改写存储的字节，所以发布快照保留原样，回滚或重新导入可以找回被跳过的节点。草稿会变，因为下一次自动保存写入的是规范化之后的图。
- 展示用的几何信息在内存中恢复：无效坐标在其作用域内得到确定性的位置，容器边界贴合子节点，不可用的边被省略，缺失或重复的边 id 被替换，无效视口回到原点，循环或悬空的视觉父节点被解除而不改变 `data.containerId`。运行分析仍使用原始的冻结图，所以恢复永远不会掩盖结构错误。
- Agent 配置被安全地投影：缺失或类型不对的值显示为空占位，不会编造执行器或模型。冻结的快照从不被改写。
- 意外的渲染失败被隔离在工作流面板内，面板提供返回操作，侧边栏仍可使用。

`deno run -A scripts/repair_workflow_snapshots.mjs --database <path>` 预览对活跃草稿（没有任何运行或发布版本引用的）的布局修复；`--apply` 在完整备份 SQLite 之后应用修复。发布的快照和被运行引用的快照从不被修改，归属不明确的情况只报告不修改。

## 节点种类

| 种类 | 作用 |
|---|---|
| `start` | 入口；声明类型化的 Start 输入和默认的运行指令 |
| `agent` | 驱动一个 Agent 会话，见[工作流运行](workflow-runs.zh.md#agent-节点) |
| `condition` | 基于类型化比较的 IF / ELSE IF / ELSE 分支 |
| `aggregator` | 把多个分支输出中第一个已赋值的作为一个变量传出 |
| `output` | 构建运行的结果对象 |
| `loop`、`loopExit` | 反馈容器及其显式退出，见[循环与迭代](loops-and-iterations.zh.md) |
| `iteration` | 遍历数组的 foreach 容器 |

编辑器目录中还有 `tool`、`junction`、`human` 和 `subflow`。运行引擎不执行它们：可达的这类节点会让发布或运行启动失败。

可执行的 Loop 图使用 `schemaVersion: 2`；扁平快照保持原来的解码行为。

### 变量聚合器

`data.aggregatorConfig.variables` 是一个有序的根选择器列表 `["nodeId", "root"]`。按声明顺序第一个**已赋值**的候选原样传出为 `{agg}.output`；已赋值指 key 存在于变量池中，所以 `null`、`false`、`0`、`""`、`[]` 和 `{}` 都算。一个都没有赋值时，运行以 `aggregator_no_match` 失败。每个候选都必须由静态的传递前驱产生（兄弟分支也算）或者是全局变量，所有候选共享一个声明类型。节点从不读取 Condition 的决定；分支选择由现有的分支投影加变量池事实表达。不支持分组、嵌套路径和类型转换。

## 不参与执行的节点

作者可以在执行路径旁边保留备用节点和连接起来的节点组。草稿、快照、回滚和导入导出都保留完整画布；执行从冻结快照派生出从入口可达的子图，不改写快照。

- 根作用域的可达性从 Start 开始；活跃的 Loop 用它的子 Start，活跃的 Iteration 用它的入口边。未使用的容器的所有成员都被排除。静态分析跟随每个 Condition 出口；运行时的分支选择与此无关。
- 备用节点从不进入调度、变量池或角色和 Skill 准备，也不产生节点运行或会话。从备用节点指向活跃节点的边被排除，所以它不会阻塞汇合；活跃节点引用备用节点的输出则校验失败。重新连上一个节点会恢复正常的校验。
- 文档级的检查仍会拒绝重复 id、悬空边、无效归属和非法的跨作用域边。
- `analyzeWorkflow` 驱动编辑器中"不参与执行"的徽标和计数以及运行总览。无法识别的种类单独报告，因为加载时已经丢弃了它们。参与关系由拓扑推导，从不持久化；分析结果绑定到文档身份，切换文档会取消过期的请求。

## Agent 节点配置

新选择的 Skill 绑定在 `skills: [{ skillId, enabled }]` 中保存 Catalog ID，界面显示名称与来源。
已有裸名称和历史 `ns:name` 在重新选择前保持原样。精确 ID 不回落同名；裸名称有多个候选时
（含不可用项）判为歧义。`namespace/identifier` 插件 ID 引用只有在插件提供一个 Skill 时才能解析。
依赖报告和卸载检查与启动预检使用相同解析规则，只有启用项计入启动告警。
不可用 Skill 会阻断启动；不可用 MCP 保持提示级。检查范围是 Ora 全局技能库，项目目录中的技能需先到 Skills 页导入。
导出将本机本地 Skill 的 ID（含已删除记录）转换为名称，
保留 `plugin:<plugin-id>:<skill-name>` 来源；本机已保存的图不变。

Agent 检查器编辑节点的执行器和模型、角色、prompt、Skill 和 MCP 绑定、结构化输出、交互性、重试和 prompt 无活动策略。各项的运行时含义见[工作流运行](workflow-runs.zh.md#agent-节点)；存储的形状如下：

- **MCP 绑定**：`mcps: [{ mcpId, enabled }]`，`mcpId` 是已安装插件的 id。只有启用的 id 构成节点会话的允许列表；没有选择（包括没有 `mcps` 的图）表示没有 MCP server。检查器列出已安装的 `kind: "mcp"` 插件；缺失的插件仍按 id 显示，可以禁用或移除。格式错误的绑定会让可执行解析失败。
- **没有配置**：工作流图是 JSON，所以 Agent 节点可以带 `"agentConfig": null`，后端会原样导入这样的文档。`null` 表示这个节点没有声明执行器、角色和绑定，与字段缺失同义：编辑器按新 Agent 节点的默认配置打开，运行页检查器不显示 Agent 区块。打开图不会把 `null` 改写成一份具体配置。
- **重试**：`agentConfig.retry = { enabled, maxRetries, initialDelaySeconds }`，`maxRetries` 为 0–5，`initialDelaySeconds` 为 0–300。缺失或为 `null` 表示 `{ enabled: true, maxRetries: 2, initialDelaySeconds: 10 }`；给出的对象必须完整。交互节点上不显示这一节。
- **prompt 无活动**：`agentConfig.promptInactivity: "timeout" | "wait" | "unbounded"`；可显式选择 `"unbounded"` 关闭静默上限。缺失或为 `null` 表示 `"timeout"`。打开旧图从不会添加这个字段。
- **结构化输出**：一个以对象为根的 JSON Schema，可以在对话框中可视化编辑或直接编辑 JSON，只有保存时才更新节点。

保存、发布、复制和导入都原样保留这些值，包括禁用的绑定；导出仅按上文转换本机 Skill ID。它们都不需要 schema 版本或数据库迁移，编辑草稿只影响之后的运行。
