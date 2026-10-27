# 工作流运行

[English](workflow-runs.md) | 中文

工作流运行冻结一个发布快照，并在调用方选定的 Workspace 中执行它：从项目行发起的运行使用项目的 Main Workspace，从任务行发起的使用该任务的 Isolated Workspace。运行的 CRUD 在 `crates/application/src/workflow_run`，执行引擎在它的 [`engine` 模块](../../crates/application/src/workflow_run/engine/README.md)，Agent 节点的会话在 `crates/backend/src/workflow/run`。UI 见 [`features/workflow-run/README.md`](../../packages/app-shell/src/features/workflow-run/README.md)。

## 模型

| 领域类型 | 数据表 | 记录 |
|---|---|---|
| `WorkflowRun` | `workflow_runs` | 固定的 `snapshot_id`、显示名称、`workspace_id`、启动 `input`、状态和运行载荷（变量池和调度器状态） |
| `WorkflowNodeRun` | `workflow_node_runs` | 一次已执行的节点尝试，带作用域、迭代轮次、会话、输出和载荷 |
| `WorkflowExecutionScope` | `workflow_execution_scopes` | 根执行和 Loop 轮次，见[循环与迭代](loops-and-iterations.zh.md#执行作用域) |

运行和节点共用 `Pending | Running | Succeeded | Failed | Cancelled`。从未开始的节点没有行；前端通过比较图节点和已记录的行推出"未开始"。等待后续输入的交互节点持久化为 `Pending`，契约把带有这种节点的运行中的运行报告为 `AwaitingInput`，让侧边栏能提示需要人来处理。

## 生命周期

| 操作 | 效果 |
|---|---|
| `createWorkflowRun` | 校验并持久化一个 `Pending` 运行 |
| `startWorkflowRun` | 校验可执行性并开始调度 |
| `updateWorkflowRunInput` | 在运行开始之前填写 Start 输入 |
| `cancelWorkflowRun`、`restartWorkflowRun` | 取消；以新的根执行从头重新开始 |
| `previewWorkflowRunResume`、`resumeWorkflowRunFromFailure` | 预览并执行恢复，见[恢复失败的运行](#恢复失败的运行) |
| `completeWorkflowNode` | 手动完成一个交互节点 |
| `diagnoseWorkflowNodeFailure` | 按需对失败节点做 AI 诊断 |
| `getWorkflowRun`、`listWorkflowRuns`、`listWorkflowRunsByWorkflow`、`listWorkflowNodeRuns` | 读取运行、按项目的摘要和节点历史 |
| `renameWorkflowRun`、`deleteWorkflowRun` | 改名；删除不在活跃状态的运行 |

### 创建

`CreateWorkflowRunHandler` 接纳请求的活跃 Workspace，解析快照（显式的 `snapshot_id` 或工作流的发布指针），检查它属于该工作流以及它声明的角色和 Skill 绑定，然后以空的 `current_nodes` 锚点持久化运行。前端传入的是打开选择器的那个项目行或任务行的确切 Workspace；backend 从不推断分支或另建 worktree。

创建时从冻结的图编译出运行级的变量池，并把启动文本写入 `{start_id}.input`，默认为快照的 Start 指令。部署界面收集运行名称、指令和已声明的 Start 输入的类型化值。Ora 的显示语言被冻结进运行，所以即使界面语言之后变了，生成的每份交接内容都使用同一种语言。

对每个使用 Skill 的节点，创建运行时向 `AgentSkillDeliveryProvider` 请求经过校验的、相对于 Workspace 的发现根目录，并冻结一份按节点的回执，内容是 Skill id、斜杠命令名和实际的包路径。本地 Skill 通过目录中的目录解析，插件 Skill 通过其行中记录的不可变包解析。当前的 provider 返回共享的 `.agents/skills` 根目录。执行只使用冻结的回执，所以目录的变化只影响新的运行。

### 事件

每次提交运行或节点运行的状态转换之后，包括交互转换（交互节点停下等待、人工回合开始或结束），backend 都发布 `AppEvent::WorkflowRunInvalidated { run_id }`。事件不携带状态；运行视图重新查询持久化的运行，轮询（运行详情每 1.5 秒）作为容忍丢失的兜底。Agent 输出走 ACP 会话流。

### 读取与删除

删除会拒绝活跃的运行（`Running` 的运行、有未终止节点的暂停运行，或绑定到它某个节点的 `Running` 会话），然后软删除运行、它的节点运行和对应的会话，并在运行拥有 Git 资源时登记持久化的 Git 清理。从未开始的 `Pending` 运行可以不经取消直接丢弃。Workspace 是共享的，从不随运行一起删除。绑定到节点运行的会话可以按 id 访问，但不出现在普通的会话列表中，即使重启后它们的行被软删除了也一样；它们永远不会变成普通聊天。绑定到已终止节点的会话是只读的。

## Agent 节点

在提示 Agent 节点之前，backend 根据冻结的图和当前节点行构建一份结构化的交接：当前步骤及其直接邻居、作为行为约束的已解析角色、按确定性拓扑顺序排列的所有节点及其状态，以及单独放置的原始运行请求。前驱的输出从不被隐式附加；只有当节点的 prompt 引用了对应变量（例如 `{{#agent-1.output#}}`）时才会收到。启用的 Skill 斜杠命令放在第一个文本块的开头（Agent CLI 按位置解析它们），后面是 Workspace 中实际的 Skill 包路径，并要求 Agent 使用这些副本。节点的 Skill 绑定是必须调用，不是允许列表。

启用结构化输出时，JSON Schema 契约追加在工作流上下文之后，只约束最终的助手回复为一个裸 JSON 对象，所以 Agent 在这一步中仍可以使用工具和修改文件。

节点的会话使用节点冻结的 MCP 选择，见 [Session MCP](../agents/session-mcp.zh.md#选择)。启用的 MCP 依赖不可用时节点显式失败；未选中的插件不会阻塞它。

`workflow_node_runs.output` 总是保存最终的助手文本，包括结构化解析或校验失败的情况。完整对话只存在于会话历史中。

### prompt 无活动

`agentConfig.promptInactivity: "wait"` 让长时间运行的 prompt 在沉默中持续等待，而不是使用默认的 45/60/90/120 秒窗口，见 [Agent 运行时](../agents/agent-runtime.zh.md#prompt-无活动与重试)。冻结的值作用于第一个回合、节点重试、Loop 和 Iteration 的各轮，以及交互节点上的后续回合。等待中的 prompt 仍处于运行状态；它既不是重试等待，也不是交互暂停。卡住的执行可能需要手动停止，真正的会话错误仍按节点的重试策略处理。要修改已有的工作流，编辑节点、发布新快照并启动新的运行。

### 交互节点

交互的 Agent 节点在第一个回合之后停下等待输入。后续消息作为人工回合在同一会话上运行，`completeWorkflowNode` 手动完成节点；一个完成认领防止后续回合与手动完成竞争。Iteration 区域的成员默认不是交互的，运行时拒绝交互成员。

每个停靠回合都会写入完成快照：turn key、stop reason、blockers 和最终 assistant 文本的 digest。只有 `EndTurn` 且没有 blocker 的回合才能正常完成；停止、截断、拒答或未知终态会保持等待输入，`completeWorkflowNode` 返回 `workflow_node_incomplete`（带上全部 blocker），除非显式传 `force`；turn key 或 digest 变化分别返回 `workflow_node_turn_changed` / `workflow_node_output_changed`，并给出 expected 与 actual。会话末尾的步骤卡片和右下角按钮是两个显式推进入口：继续、二次确认后的强制完成，或卡片 ready 时的完成并推进。

workflow 自己发起的首轮回合本身从不读作 ready：它的 `EndTurn` 只表示 agent 结束了这次回复，而这次回复可能在向用户提问。快照因此带 `turn_unconfirmed` blocker，卡片保持「等待参与」徽标，只提供「继续当前步骤」与「仍然完成（采用当前产出）」（后者需要确认并留痕），右下角推进按钮在用户回复或确认前不可用。由用户回复开启的回合仍按上面的规则：`EndTurn` 且没有 blocker 时正常完成，卡片重新给出「完成并进入下一步」。

如果流在 stop reason 持久化前被 drop，节点按最后一次可读历史写 provisional 快照：保留它描述的 `userSeq`，并带 `turn_not_settled` warning；之后完成时，只要同一 user 消息的回合已经收敛就修复该快照。升级前停靠、完全没有快照的节点由第一次完成尝试在读取会话历史后物化快照，再走同一套 blocker 门禁，所以未完成的回合同样会被拒绝，不会静默推进。

每个人工回合都带一个从冻结图渲染的 `<workflow_step_boundary>` 块：它写明当前步骤与直接后续节点，并声明只完成本步骤；它不承诺运行时会阻止后续节点，而记录给用户的 prompt 仍是用户原文。

`force` 完成会在节点 payload 的 `completion_forced` 下留痕，记录被绕过的 blockers、本轮是否有终态，以及采用的输出从哪来（`turn_window` 或 `last_assistant_message`）。这条审计与完成在同一个事务里写入，所以已完成的节点不会缺这条记录。

## 变量与类型

运行的变量池保存在 `workflow_runs.payload` 中。它的目录声明类型化的 Start 输入和产生数据的节点的稳定输出；启用结构化输出时还声明 `agent-1.structured_output`。`variablePool.values` 只保存已赋值的值。节点完成时的写入与节点状态转换在同一个事务里提交。启动指令留在 `workflow_runs.input` 中，不是可选择的变量。

- Condition 节点不暴露变量，也不持久化输出；它们选中的分支是调度器的私有状态（`conditionDecisions`），重启后仍保留。它们对作用域是透明的：下游节点看到的是 Condition 的直接前驱的变量。
- 编辑器为每个节点提供工作流全局变量和每个直接前驱的变量。其他祖先不可用，除非被转发。结构化输出的字段路径出现在同一目录中，所以选择器从不依赖自由文本。
- 自定义全局变量需要显式的带点名称和类型正确的初始值；系统全局变量归运行时所有。
- 每个终止的 Output 构建自己的结果对象；结果名只需在该节点内唯一。

类型在图解析、编辑器输入和变量池写入时强制执行。`array` 和 `array[any]` 接受混合类型的数组；带类型的数组校验每个元素。`file` 是 `{ "kind": "workspace_file", "path": "relative/path" }`，`array[file]` 是它的列表；绝对路径、父目录遍历、空路径和平台保留路径都被拒绝。结构化 Agent schema 使用相同的类型，被递归校验，并在 prompt 中生成一个合法的 JSON 示例。

Start 输入把表单控件与变量池类型分开：文本、段落和下拉框产生 `string`，数字产生 `number`，复选框产生 `boolean`，单文件产生 `file`，文件列表产生 `array[file]`，JSON 可以产生任意结构化类型。选项、必填和长度限制被冻结进快照，并在执行时再次检查。被拒绝的值以 `workflow_run_input_invalid` 报告，指明变量和原因；一次提交只报告按字母顺序的第一个拒绝，且不应用任何值。

## 失败

节点失败时，运行立即失败，正在进行的兄弟节点继续运行到结束；之后调度器不再派发任何东西。失败的节点运行记录 `payload.error_detail`：`kind`（机械的分类，从不由模型推断）、`message`、`source_chain`、`attempt`、`resumable`、`injects_previous_failure`、`auto_retryable` 和 `recorded_at`。

| 类型 | 可恢复 | 自动重试 |
|---|---|---|
| `session`、`session_ended_without_stop_reason`、`session_binding_rejected` | 是 | 是 |
| `workflow_model_not_found`、`missing_agent_config`、`interrupted_by_restart`、`repository`、`baseline_persist` | 是 | 否 |
| `structured_output`、`agent_refusal`、`unknown_stop_reason` | 否 | 是 |
| `multiple_outputs` | 否 | 否 |
| `prompt_template`、`missing_agent_ref`、`missing_skill_materialization`、`invalid_run_payload`、`condition_evaluation`、`aggregator_no_match` | 否 | 否 |

`resumable` 预测用同一快照重跑是否是合理的第一步；对失败或已取消的空闲运行，恢复总是可用的。只有 Agent 行为类的失败（`structured_output`、`agent_refusal`、`unknown_stop_reason`、`multiple_outputs`）会注入到下一次尝试的 prompt 中，并且只在运行级开关 `inject_last_failure` 开启时（默认开启）；渲染出的内容保存为 `payload.injected_failure_context`。

### 自动重试

只有 Agent 节点会重试，包括 Iteration 区域和 Loop 体内的 Agent，交互节点从不重试。第 `n` 次重试等待 `initialDelaySeconds × 2^(n-1)` 秒，上限 600 秒；默认策略等待 10 秒和 20 秒，所以一个节点最多运行三次尝试。

被覆盖的失败会记录失败尝试的 `error_detail`，软删除那次尝试，并在同一个运行、作用域和轮次中插入下一次尝试，作为等待中的行（`Running`，没有 `started_at`，`payload.retry_wait` 中带有尝试序号、延迟和 `due_at`）。因为这一行是 `Running`，运行保持运行中，独立的分支继续，后继节点等待。`due_at` 到达时，一个计时器在运行锁下唤醒引擎，引擎重新读取标记，并在原来失败的作用域中派发这次尝试；过早或过期的唤醒什么也不做。重试之前不回滚，这次尝试会自己做检查点。会话内因沉默触发的 prompt 重发是另一回事；`"wait"` 只关闭那个，不关闭真实失败之后的重试。

`payload.auto_retry = { retry, max_retries }` 计算预算；第一次尝试、恢复的尝试或重启之后的尝试重新开始计算，而 `error_detail.attempt` 继续计数（三次失败后恢复，接下来运行第 4、5、6 次尝试）。在 Loop 中，两者在每一轮都重新开始。等待会在以下情况提前结束：取消（行变为 `Cancelled`）、运行失败（行变为 `Cancelled`，带 `{"reason":"retry_abandoned"}`）、重启（启动扫尾让外层的等待行以 `interrupted_by_restart` 失败；运行中的 Iteration 内的行让它的轮次以失败结算）。

`getWorkflowRun` 暴露等待中的行和 `failedAttempts`：更早被软删除的 `Failed` 尝试及其 `error_detail`，按时间先后，`sourceChain` 中包括 Agent 自己给出的原因。尝试序号可能跳号，因为被取消或放弃的尝试以及没有失败记录的尝试会占用序号但不列出。运行视图把等待中的行显示为"等待重试"并每秒倒计时，列出更早的尝试以及每次是如何被替代的（自动重试、手动恢复、在"从头再次运行"之前），并在某种失败类型从不重试时加以说明。

### 恢复失败的运行

恢复会软删除失败或已取消的节点运行及其所有后代，然后从剩下的状态重新调度。尝试按 `(run, node, iteration)` 编号，Loop 体内的行按 Loop 轮次编号。容器内任何东西的恢复单位都是所属的 Loop 或 Iteration，它从第 1 轮重新开始；不支持在容器内部分恢复。当两张图兼容时，运行可以在更新的发布快照上恢复；删除的节点、改变的节点类型和 Start 契约变化会被报告（`node_missing:<id>`、`node_type_changed:<id>`、`start_node_changed`、`start_variables_changed`、`variable_type_changed:<selector>`、`variable_missing:<selector>`），恢复单位之外已完成的 Iteration 不能改变配置或成员。节点最后运行的快照记录为 `payload.snapshot_id`。

### 回滚

每个节点在运行之前在 `refs/ora/checkpoints/<node_run_id>` 下记录一个 Git 检查点（见 [Gitlancer](../workspaces/gitlancer.zh.md)），在载荷中记录 `checkpoint`、`checkpoint_error` 和 `file_changes`。恢复提供三种回滚方式；回滚之前，Workspace 会被快照为 `pre-rollback-<run>-<ts>`，以便撤销。

| 方式 | 效果 | 不可用的情况 |
|---|---|---|
| `keep` | 保持 Workspace 不变 | 从不 |
| `node_files` | 只恢复失败节点改动的路径 | `no_file_changes`；恢复单位是容器时为 `composite_region` |
| `checkpoint` | 把整个 Workspace 恢复到恢复单位的检查点 | `no_checkpoint`、`siblings_ran_after_checkpoint`、`not_resumable` |

被自动重试替代的节点与它重试链上的每次尝试（`payload.retry_chain`）作为一个单位回滚。按需的 AI 诊断写入 `payload.ai_diagnosis`，标注为推测，调度、恢复、回滚和快照切换从不读取它。

## 重启后的恢复

Desktop 只有在持有应用实例锁之后才运行启动扫尾（见 [Desktop 宿主](../desktop/desktop-host.zh.md#启动)），所以存活的第二个进程永远不会把运行中的工作标记为已中断。被中断的外层行以 `interrupted_by_restart` 失败并让运行失败；运行中的容器内被中断的行失败，容器和运行保留，该轮以失败结算。合法的交互等待被保留，副作用未知的工具从不自动重放。
