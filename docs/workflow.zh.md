# 工作流

[English](workflow.md) | 中文

已交付扩展：[工作流循环节点实现计划与证据](workflow-loop-plan.zh.md)。

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

## Loop 容器

可执行 Loop 图使用 `schemaVersion: 2`。根 Loop 持有 `data.loopConfig`；每个子节点通过
React Flow `parentId` 与 `data.containerId` 指向同一个 Loop。编辑器一次创建包含唯一子
Start、子 Agent 和内部边的合法容器组。根图与子图禁止跨作用域连线；删除 Loop 会原子
删除后代及相关边；根图自动布局保留子节点的相对位置。编辑器与运行全图都会把所属节点
渲染在 Loop 体内；编辑器中的子节点受容器边界约束，选中 Loop 后可调整其大小，保存的
尺寸会继续用于发布快照和运行全图。

`loopConfig` 定义 1–100 的轮次上限、有类型跨轮变量、同时反馈选择器、有类型 `until`
条件及命名输出。每个 Loop 体是独立 DAG，必须有且仅有一个可达 Start。嵌套 Loop、归属
不一致、跨作用域边或选择器、类型错误及不可达子节点都会在创建 Session 前被拒绝。编辑器
默认组把子 Agent 输出反馈为下一轮 `value`，输出非空时结束并导出为 `result`；作者可设置
初始值与最大轮次。

每次迭代拥有持久化 `WorkflowExecutionScope`。子 NodeRun 与 Session 归属于该作用域，重复的
定义节点 ID 不会覆盖其他轮次。轮次完成后，从已完成变量池解析反馈和终止条件，并在同一
仓储事务中创建下一作用域或完成父 Loop。取消与子节点失败会同时收束活跃作用域和父节点。
重跑会轮换根执行身份，保留旧历史并使迟到回调失效。真实运行契约返回有序作用域身份，
Theater 的 Loop 详情可切换轮次，查看各轮子节点状态与 Session ID。

## Agent 节点 MCP 绑定

节点配置读取已安装的 `kind: "mcp"` 插件，展示名称、完整 ID 和配置可用性。每个节点独立添加、启用、禁用和移除绑定，添加后默认启用。加载失败时提供重试并保留现有配置；插件缺失时仍显示 ID，允许禁用或移除。

安装 MCP 只会把插件加入全局可选目录，不会写入工作区文件；节点中启用的绑定构成该节点 Session 的严格白名单。

图中使用 `mcps: [{ mcpId, enabled }]` 保存绑定，其中 `mcpId` 是完整插件 ID。草稿保存、发布、复制和导入导出均保留绑定，包括已禁用项。空列表或旧图缺少 `mcps` 时表示节点未授权任何 MCP。可执行图解析会拒绝格式错误的绑定。

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

运行通过 `workspace_id` 绑定现有工作区。Agent 节点在选定工作区执行，每个节点创建独立会话，冻结的执行配置决定 Agent、模型、角色、Skill、MCP 和提示词。运行 CRUD 层与图内容无关；执行引擎在同一仓库之上负责启动/重启/HITL，节点执行策略按节点类型注册在运行时注册表后面——Start/Condition/Output 是调度波内同步完成的快执行器，`ora-backend` 的 `WorkflowRunNodeExecutor` 被包装为 Agent 运行时，引擎核心只保留调度职责。每次提交 run 或节点运行状态转移后，引擎在应用事件流上发布 `AppEvent::WorkflowRunInvalidated { run_id }`；事件不携带任何工作流状态，前端运行视图收到后重新查询持久化的运行详情与列表，而不是把事件负载当作渲染事实源。引擎之外的交互转移——交互节点首轮结束后停靠等待输入、人工追问开始与结束——经共享的转移提交入口在同一通道上发布，因此每次提交的节点运行转移都可观察。Agent 输出的实时流仍走 ACP 会话流；失效事件只标记状态变化，运行视图的轮询作为容忍事件丢失的兜底保留。

Skill 由 Effect 默认物化到所有符合条件的工作区；节点中启用的 Skill 表示该节点必须调用，并不构成安全白名单，也不会阻止 Agent 看到同一工作区内的其他 Skill。部署时会校验必需 Skill，运行保存包含调用名称和包路径的物化回执。调用名称和落点来自该回执，执行节点不再按名称重新读取可变的全局目录。Skill 交付通过 `AgentSkillDeliveryProvider` 隔离，发现根目录的数量和位置可随 Agent 实现变化，不必改变工作流创建或提示词组装逻辑。

节点完整对话以会话历史为唯一来源。`workflow_node_runs.output` 保存最终助手文本，用于展示、审计及 `agent-1.output` 访问；即使结构化解析或校验失败也保留原文。成功校验的结构化对象才写入 `agent-1.structured_output`。

运行变量池保存在 `workflow_runs.payload`。它包含有类型的 Start 输入和产出数据节点的稳定输出，`variablePool.values` 只保存已赋值变量。Condition 不公开输出变量，其分支选择单独保存在 `conditionDecisions`，能够跨重启恢复。完成节点的变量写入与状态切换在同一 SQLite 事务中提交。

运行的启动指令保存在 `workflow_runs.input`，不会作为普通工作流变量供选择。Start 输入可以在定义中暂不赋值，部署前填写。自定义全局变量必须包含显式点分名称与类型正确的初始值；系统全局变量由运行时拥有。每个节点可使用全局变量与直接前驱变量，Condition 对变量作用域透明，连续 Condition 会传递原始前驱变量。其他间接祖先变量必须通过直接前驱转发。结构化字段路径进入同一变量目录，Condition 和 Output 无需手写自由文本选择器。各终端 Output 独立构建结果对象，结果名只需在同一 Output 内唯一。

变量类型在图解析、编辑器输入和变量写入时检查。`array` 与 `array[any]` 允许异构数组；有类型数组检查每个元素。文件使用工作区相对引用 `{ "kind": "workspace_file", "path": "relative/path" }`，文件数组使用该对象数组。旧路径字符串会被规范化；绝对路径、父目录穿越、空路径和平台保留路径被拒绝。结构化 Agent Schema 递归校验，并在提示词中生成有效示例。Agent 输出的文件字段必须遵循该对象形式。

Start 表单控件与变量类型分离：文本、段落、选择框、数字、复选框、单文件、文件列表和 JSON 分别产出 `string`、`string`、`string`、`number`、`boolean`、`file`、`array[file]`、`object`。展示名称、选项、必填项和文本长度限制随快照冻结，并在部署边界重新校验。旧快照按变量类型推导兼容控件。

### 迭代节点（foreach 复合运行时）

迭代节点是第一个复合运行时：它拥有一个区域——`parentId` 指向它的全部节点——并对数组源的
每个元素执行一轮区域内的冻结子图。`data.iterationConfig` 携带 `iteratorSelector`（数组类型
变量）、`collectSelector`（区域内声明的根变量）、`errorStrategy`（`fail` 或 `continue`）与
`maxIterations`（默认 50）。区域边界在图解析期校验：区域必须非空且由迭代节点的边进入、不得
包含 Output 或嵌套复合节点、成员出边不得离开区域、外层节点不得连向成员、`maxIterations`
至少为 1。

轮次是持久化事实。区域行在 `iteration` 列携带其轮次（迭代节点自身的行为 NULL），同一节点
每轮一行。每轮的 `{iter}.item` / `{iter}.index` 绑定与该轮首批节点行在同一 SQLite 事务提交；
每个已结算轮次的账本条目与其后续转移（下一轮、节点完成或节点失败）也在同一事务提交。引擎
不为迭代持有内存态：当前轮恒从区域行重推导，崩溃恢复因此可以重放到同一点。开机清扫感知
区域——仍在运行的迭代区域内部被打断的行标记 `interrupted_by_restart`，而复合行与 run 存活，
运行时随后把被打断的轮次结算为失败的账本条目。

错误处理解耦为二值控制流开关加按轮账本。`fail`（默认）在首个失败轮终止并使 run 失败；
`continue` 把失败轮记入账本并继续执行剩余轮。完成时节点暴露三个类型不随策略改变的变量：
`{iter}.output`（`array[T]`，T 为收集目标的声明类型）、`{iter}.entries`（`array[object]`，每轮
一个 `{item, status, output, error}` 信封，与输入数组位置对齐）与 `{iter}.failed_count`
（`number`）。迭代源长度超过 `maxIterations` 时节点在启动边界失败——绝不静默截断——错误信息
包含长度与上限；空源立即成功完成且输出为空。分支绕开收集目标的轮次按失败结算
（`collect target did not run this round`），而不是读取上一轮的陈旧池值。迭代节点自身的失败
总是传播为 run 失败；只有区域内部失败可被吸收，且区域内的 Condition 决策按轮记录，后一轮
永远不会覆盖前一轮的分支选择。

编辑器把迭代节点渲染为同一画布上的内嵌复合区域：顶部是紧凑标题栏，参数仍在 Inspector 配置；
区域左侧固定显示一个不可删除、不可配置的内部起点，样式参照 Dify 的迭代开始节点：44 像素
白色圆角卡片内嵌蓝色家园徽标，右边缘带入口端口。参照 Dify，新增节点的入口是一个实心蓝色
圆圈加号徽标，居中压在端口上：仅在作者悬浮节点时淡入（菜单展开或徽标获得键盘焦点期间保持
可见），而起点卡片与端口本身就是选择器触发器——点击起点卡片或端口都会打开节点选择器，
从端口拖动仍会发起新连线，装饰性徽标永远不会拦截拖动。作者既可从内部起点这样新增 Agent 或 Condition，也可从尚未连接
的成员输出（包括指定的 Condition 分支）追加节点，还可从起点拖线连接已有成员并创建多个入口
分支；内部连线上也可插入节点。入口边不再重复显示中点“+”，因为固定起点
已经拥有该新增入口；内部画布只保留容器本身的一层边界，不再绘制第二层虚线框。选中区域仅
重绘边框与阴影，背景填充在选中与未选中状态下保持不变。内部起点只属于
编辑器呈现，冻结图仍把入口保存为 `iteration --iteration-entry--> member`，不会新增运行时节点。
每次插入会在同一次撤销/自动
保存操作中创建或重接连线。拖动永远不改写 `parentId`：成员只能在所属区域内移动；外层节点落到
区域上时回到原位置，并提示使用区域内新增入口。React Flow 的父级约束只在渲染时从 `parentId`
派生，不进入持久化图。

展开区域以 560×340 为最小尺寸，通过 `initialWidth` / `initialHeight` 保存适配后的尺寸。作者
也可以像 Dify 一样手动缩放展开的区域：右下角显示柔和的灰色弧线角标（悬浮区域或区域被选中时
显现），光标移入该角落变为缩放指示，按住左键拖动即可按 20 像素网格步长调整尺寸；该手势是一次
可撤销、自动保存的编辑，且永远不会低于最小尺寸或裁剪区域成员。折叠时
内部起点、成员和内部连线仅在画布投影中隐藏，原始图不变；重新展开会恢复同一尺寸。新增、插入或
移动成员时容器扩展；删除或自动整理时紧凑计算。由于新插入的卡片只有在渲染后才能测得真实尺寸，
真实测量到达时会重新拟合容器，确保 React Flow 的父级约束不会把较高的成员钳回区域内部操作行之
上；新成员堆叠在既有成员真实底部之下，并会被推到与其重叠的卡片下方，因为卡片高度随节点类型与
内容变化。自动整理先分别
排布每个区域内部 DAG，再使用容器的真实尺寸排布外层图。删除非空区域前会显示成员数量；确认后把
容器、成员和相关边作为一次可撤销编辑级联删除。若删除当前 `collectSelector` 的目标，编辑器会清空
该选择器并提示重新配置。区域内 Agent 默认 `interactive: false`；旧快照中的非法交互成员仍会显示，
并提供直接关闭交互模式的修复操作，因为运行时仍会拒绝它们。

变量目录遵循区域作用域——成员可见 `item` / `index` 与区域内上游产物，但看不到节点自身暴露的
结果；外层消费者可见三个暴露变量，但看不到轮内绑定。运行视图按 `(node_id, iteration)` 分组区域
状态。剧场的顶层路径只保留迭代容器，并在其下按冻结区域 DAG 展开内部结构：多个
`iteration-entry` 目标始终显示为并行组，Condition 后继标为条件分支，存在依赖的节点进入后续阶段。
整个区域共用一个轮次选择器；切换并行成员会保持当前轮次，成员未在该轮执行时明确显示“本轮未执行”，
绝不借用其他轮次的结果。节点详情上方持续显示所属迭代、轮次和并行位置。缺少按轮投影的旧运行记录
仍按冻结图分组，并使用节点级状态。总览会从冻结图的 `parentId`、`initialWidth`、`initialHeight` 与
`iteration-entry` 边重建迭代父框，
完成后的成员节点仍留在区域内，入口边也保持可见。总览支持鼠标滚轮、触控板捏合、加减按钮和
“显示完整运行图”缩放操作。
生产回归测试通过 SQLite 与 fake ACP provider 覆盖同一组边界：第二轮会获得新的会话，轮次绑定
在提示词渲染时已经可用，同步失败在 `fail` 与 `continue` 两种策略下都会完成结算，不会留下卡住的 run。

### 失败可见性与从失败处续跑

失败节点保持可见。某个节点失败时，运行立即失败（D2），仍在执行的兄弟节点会跑完且仍可绑定；
调度器随后不会再向 `Failed` / `Cancelled` 运行派发新节点。失败节点的
`payload.error_detail` 记录 `kind`、`message`、`source_chain`、`attempt`、`resumable`、
`injects_previous_failure`、`recorded_at`。`kind` 是机械分类，从不由模型推断。

`resumable` 只表示「同一快照再跑一次是否像环境/瞬时问题」，不决定界面是否允许续跑——失败或
已取消且空闲的运行始终可以续跑：

| Kind | `resumable` |
| --- | --- |
| `workflow_model_not_found`、`missing_agent_config`、`session`、`session_ended_without_stop_reason`、`session_binding_rejected`、`interrupted_by_restart`、`repository`、`baseline_persist` | true |
| `structured_output`、`agent_refusal`、`prompt_template`、`missing_agent_ref`、`missing_skill_materialization`、`invalid_run_payload`、`unknown_stop_reason`、`multiple_outputs`、`condition_evaluation` | false |

只有智能体自身行为导致的失败会注入后续提示词（`injects_previous_failure`）：
`structured_output`、`agent_refusal`、`unknown_stop_reason`、`multiple_outputs`。

续跑会软删除失败/取消的节点运行及其全部后继（`is_deleted = 1`），再从幸存状态重新调度。
尝试次数按 `(run_id, node_id, iteration)` 统计软删除前驱（外层行为 `iteration IS NULL`）。
`find_last_failed_attempt` 使用同一作用域。

每个节点开始前会在 `refs/ora/checkpoints/<node_run_id>` 记录 git 检查点。回滚前先把工作树
存成 `pre-rollback-<run>-<ts>`，方便反悔。节点 payload 保存 `checkpoint`、
`checkpoint_error`、`file_changes`。三种回滚模式：`keep`（保留现状）、`node_files`
（只还原失败节点记录过的路径）、`checkpoint`（把整棵工作树还原到续跑单元的检查点）。
`node_files` 在失败节点没有检查点或文件改动时不可用（`nodeFilesUnavailableReason` 为
`"no_file_changes"`），续跑单元是迭代复合节点时也不可用（`"composite_region"`）。
`checkpoint` 不可用的原因是 `"no_checkpoint"`、`"siblings_ran_after_checkpoint"`（续跑单元
最早开始之后，单元外仍有活着的节点在跑：`finished_at` 为空或更晚，或 `started_at` 更晚；
在该时刻之前已结束的 Start/Condition/Output 行不算），或 `"not_resumable"`。

运行级开关 `inject_last_failure`（默认开启）会在同一 `(node_id, iteration)` 的上次失败属于
上述四种可注入 kind 时，把失败信息写入提示词，并保存在
`payload.injected_failure_context`。

失败或已取消的运行可以改用更新的已发布快照续跑，前提是两张图兼容：删除节点、改变节点类型、
改变 Start 契约都不兼容（`node_missing:<id>`、`node_type_changed:<id>`、
`start_node_changed`、`start_variables_changed`、`variable_type_changed:<selector>`、
`variable_missing:<selector>`）。已成功且不在续跑单元内的迭代复合节点，若
`iterationConfig` 或区域成员集合变了，也不兼容（`iteration node <id> changed after it
completed`）；本身就是续跑单元的复合节点可以任意改，因为它会从第一轮重跑。节点上次实际运行
的快照记在 `payload.snapshot_id`。

按需 AI 诊断写入 `payload.ai_diagnosis`。它标明为推测，调度、续跑、回滚、快照切换都不会读取。

区域内任何失败/取消行（`iteration IS NOT NULL`），或复合节点自身失败/取消，都以拥有该区域
的复合节点为续跑单元：软删除复合行、每一轮的全部区域行、该复合节点写入的账本与池绑定
（`{iter}.item`、`{iter}.index`，以及已暴露的 `{iter}.output` / `{iter}.entries` /
`{iter}.failed_count`），以及复合节点的全部外层后继，然后重新调度；循环从第 1 轮重来。
不支持循环内部分续跑。该单元不能使用 `node_files` 回滚（`composite_region`）；
`checkpoint` 还原到循环开始前为复合节点记录的检查点。开机清扫仍感知区域：被打断的区域行
记 `interrupted_by_restart`，复合行与运行存活，该轮按失败结算；非区域行仍走整次运行的
`InterruptedByRestart` 处理。

Loop 容器（`kind: "loop"`，见「Loop 容器」）按同样方式续跑：Loop 节点本身是续跑单元，清除它时
会一并关闭其各轮作用域并软删除这些轮次创建的全部节点记录，因此重跑从第 1 轮开始、没有遗留的
活跃轮次。轮次内部沿用 Loop 自己的失败语义（同一轮的兄弟节点被取消，失败上抬到 Loop 节点）；
D2 的「兄弟节点继续跑完」只适用于根作用域。

### 实体与状态

| 领域类型                 | 数据表                      |
| ------------------------ | --------------------------- |
| `WorkflowRun`            | `workflow_runs`             |
| `WorkflowNodeRun`        | `workflow_node_runs`        |
| `WorkflowExecutionScope` | `workflow_execution_scopes` |

`WorkflowRun` 固定引用发布版本的 `snapshot_id`，保存运行名称与工作区。`WorkflowNodeRun` 只为实际开始的节点创建记录并保存作用域归属，未开始状态由前端对比图与记录推导。`WorkflowExecutionScope` 保存 Loop 父执行、轮次索引、生命周期与私有轮次状态。

运行与节点均使用 `Pending | Running | Succeeded | Failed | Cancelled`。交互节点等待后续输入时持久化为 `Pending`；公共契约将存在等待节点的运行投影为 `AwaitingInput`。终态节点会话只读，后端拒绝新提示词。节点会话可按 ID 读取，但不会出现在普通聊天列表中。过滤依据包括重跑时被软删除的节点记录：工作流完成、重跑或应用重启都不会改变会话归属，保留的节点会话历史不会成为普通聊天。

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
