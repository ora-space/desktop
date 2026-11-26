# 工作流运行

[English](workflow-runs.md) | 中文

工作流运行冻结一个发布快照，并在调用方选定的 Workspace 中执行它：从项目行发起的运行使用项目的 Main Workspace，从任务行发起的使用该任务的 Isolated Workspace。运行的 CRUD 在 `crates/application/src/workflow_run`，执行引擎在它的 [`engine` 模块](../../crates/application/src/workflow_run/engine/README.md)，Agent 节点的会话在 `crates/backend/src/workflow/run`。UI 见 [`features/workflow-run/README.md`](../../packages/app-shell/src/features/workflow-run/README.md)。

## 模型

| 领域类型                 | 数据表                      | 记录                                                                                               |
| ------------------------ | --------------------------- | -------------------------------------------------------------------------------------------------- |
| `WorkflowRun`            | `workflow_runs`             | 固定的 `snapshot_id`、显示名称、`workspace_id`、启动 `input`、状态和运行载荷（变量池和调度器状态） |
| `WorkflowNodeRun`        | `workflow_node_runs`        | 一次已执行的节点尝试，带作用域、迭代轮次、会话、输出和载荷                                         |
| `WorkflowExecutionScope` | `workflow_execution_scopes` | 根执行和 Loop 轮次，见[循环与迭代](loops-and-iterations.zh.md#执行作用域)                          |

运行和节点共用 `Pending | Running | Succeeded | Failed | Cancelled`。从未开始的节点没有行；前端通过比较图节点和已记录的行推出"未开始"。等待后续输入的交互节点持久化为 `Pending`，契约把带有这种节点的运行中的运行报告为 `AwaitingInput`，让侧边栏能提示需要人来处理。

## 生命周期

| 操作                                                                                       | 效果                                                |
| ------------------------------------------------------------------------------------------ | --------------------------------------------------- |
| `createWorkflowRun`                                                                        | 校验并持久化一个 `Pending` 运行                     |
| `startWorkflowRun`                                                                         | 校验可执行性与依赖并开始调度                        |
| `updateWorkflowRunInput`                                                                   | 在运行开始之前填写 Start 输入                       |
| `cancelWorkflowRun`、`restartWorkflowRun`                                                  | 取消；以新的根执行从头重新开始                      |
| `previewWorkflowRunResume`、`resumeWorkflowRunFromFailure`                                 | 预览并执行恢复，见[恢复失败的运行](#恢复失败的运行) |
| `completeWorkflowNode`                                                                     | 手动完成一个交互节点                                |
| `diagnoseWorkflowNodeFailure`                                                              | 按需对失败节点做 AI 诊断                            |
| `getWorkflowRun`、`listWorkflowRuns`、`listWorkflowRunsByWorkflow`、`listWorkflowNodeRuns` | 读取运行、按项目的摘要和节点历史                    |
| `renameWorkflowRun`、`deleteWorkflowRun`                                                   | 改名；删除不在活跃状态的运行                        |

### 创建

`CreateWorkflowRunHandler` 接纳请求的活跃 Workspace，解析快照（显式的 `snapshot_id` 或工作流的发布指针），检查它属于该工作流以及它声明的角色和 Skill 绑定，然后以空的 `current_nodes` 锚点持久化运行。前端传入的是打开选择器的那个项目行或任务行的确切 Workspace；backend 从不推断分支或另建 worktree。

创建时从冻结的图编译出运行级的变量池，并把启动文本写入 `{start_id}.input`，默认为快照的 Start 指令。部署界面收集运行名称、指令和已声明的 Start 输入的类型化值。Ora 的显示语言被冻结进运行，所以即使界面语言之后变了，生成的每份交接内容都使用同一种语言。

对每个使用 Skill 的节点，创建运行只冻结按节点的回执 `{ node_id, skill_id, invocation_name }`——不冻结内容、版本或路径。节点派发时按 `skill_id` 重新解析当前正式存储中的 `SKILL.md` 绝对路径（本地 Skill 在 `<数据根目录>/atoms/skills/<name>/SKILL.md`，插件 Skill 在行记录校验后的 `package_root` 内），并忽略升级前旧运行里冻结的工作树相对 `packagePaths`。运行中途升级 Skill 后，后续节点读到新版本路径；若中途卸载，被派发的节点直接以 `workflow_skill_not_found` 失败，不回退到其他同名 Skill 或旧路径。

启动、重启或续跑在派发节点前，用当前角色与 Skill 目录（要求各 `skill_id` 能解析为正式存储 `SKILL.md` 存在且可解析的 Available Skill）、已安装的 MCP 包检查即将执行的整张图。创建运行时校验角色和 Skill；启用的 MCP 绑定在开始调度时需要有效的 MCP 包。Skill 或 MCP 不可用时，`workflow_skill_not_found` 或 `workflow_mcp_not_found` 指出节点及其缺失的依赖，恢复依赖后同一运行可继续。对运行中的工作流再次启动会返回当前运行，重启则返回 `workflow_run_not_restartable`。Agent 是否安装不属于这项预检：runtime 在节点打开会话时解析冻结的 Agent 身份，缺失时以 `agent_not_installed` 失败。

续跑切换快照时，会在恢复检查点或改动工作树之前检查目标图，切换时只为目标快照冻结 `{ node_id, skill_id, invocation_name }`。保留快照时继续使用已有的冻结回执。派发后，节点会话建立时处理此时已移除或配置不完整的 MCP 以及 Agent 能力不足等情况，拒绝交付不完整的选择。任何 Agent 都可执行带 Skill 的节点而无需声明 Skill 交付能力；只有同一运行中两个不同 Skill 解析出相同调用名时仍在启动时拒绝。

### 事件

每次提交运行或节点运行的状态转换之后，包括交互转换（交互节点停下等待、人工回合开始或结束），backend 都发布 `AppEvent::WorkflowRunInvalidated { run_id }`。事件不携带状态；运行视图重新查询持久化的运行，轮询（运行详情每 1.5 秒）作为容忍丢失的兜底。Agent 输出走 ACP 会话流。

### 读取与删除

删除会拒绝活跃的运行（`Running` 的运行、有未终止节点的暂停运行，或绑定到它某个节点的 `Running` 会话），然后软删除运行、它的节点运行和对应的会话，并在运行拥有 Git 资源时登记持久化的 Git 清理。从未开始的 `Pending` 运行可以不经取消直接丢弃。Workspace 是共享的，从不随运行一起删除。绑定到节点运行的会话可以按 id 访问，但不出现在普通的会话列表中，即使重启后它们的行被软删除了也一样；它们永远不会变成普通聊天。绑定到已终止节点的会话是只读的。

## Agent 节点

在提示 Agent 节点之前，backend 根据冻结的图和当前节点行构建一份结构化的交接：当前步骤及其直接邻居、作为行为约束的已解析角色、按确定性拓扑顺序排列的所有节点及其状态，以及单独放置的原始运行请求。前驱的输出从不被隐式附加；只有当节点的 prompt 引用了对应变量（例如 `{{#agent-1.output#}}`）时才会收到。启用的 Skill 斜杠命令放在第一个文本块的开头（Agent CLI 按位置解析它们），后面是在派发时解析出的正式存储 `SKILL.md` 绝对路径，并指示 Agent 读取并遵循其中说明。节点的 Skill 绑定是必须调用，不是允许列表。

启用结构化输出时，JSON Schema 契约追加在工作流上下文之后，只约束最终的助手回复为一个裸 JSON 对象，所以 Agent 在这一步中仍可以使用工具和修改文件。

节点的会话使用节点冻结的 MCP 选择，见 [Session MCP](../agents/session-mcp.zh.md#选择)。启用的 MCP 依赖不可用时节点显式失败；未选中的插件不会阻塞它。

`workflow_node_runs.output` 总是保存最终的助手文本，包括结构化解析或校验失败的情况。完整对话只存在于会话历史中。

### prompt 无活动

工作流 Agent 选择「持续等待」后，连续 30 分钟无进展时本轮失败。节点可显式选择无限等待。非空输出、同来源计划变化、工具状态或输出变化，以及已写入的父工具权限应答会重新计时。工具一直处于执行中也会到期。静默不会触发重发。取消已确认或旧通道已隔离后，非交互节点按策略重试。交互自动首轮直接失败，人类回合保留会话和历史并异常停靠。冻结的限时用于各次节点尝试、循环和交互追问。见[Agent 运行时](../agents/agent-runtime.zh.md#prompt-无活动与重试)。

### 交互节点

交互的 Agent 节点在第一个回合之后停下等待输入。后续消息作为人工回合在同一会话上运行，`completeWorkflowNode` 手动完成节点；一个完成认领防止后续回合与手动完成竞争。Iteration 区域的成员默认不是交互的，运行时拒绝交互成员。

每个停靠回合都会写入完成快照：turn key、stop reason、blockers 和最终 assistant 文本的 digest。新建或重新保存的交互 Agent 节点默认 `completionSignal: agent`：host 向该节点会话注入 loopback 完成工具，只有 agent 为结算的 attempt 调用它、并且用户确认（或节点配置了 `autoAcceptAgentCompletion`）后节点才完成。本轮没有信号时快照带 `agent_signal_pending`，有信号但未确认时带 `agent_confirmation_pending`，两者与其他 blocker 一起物化进停靠快照。`completionSignal: end_turn` 的遗留图仍按旧规则：只有 `EndTurn` 且没有 blocker 的回合才能正常完成。停止、截断、拒答或未知终态会保持等待输入，`completeWorkflowNode` 返回 `workflow_node_incomplete`（带上全部 blocker），除非显式传 `force`；turn key 或 digest 变化分别返回 `workflow_node_turn_changed` / `workflow_node_output_changed`，并给出 expected 与 actual。会话末尾的步骤卡片和右下角按钮是两个显式推进入口：继续、二次确认后的强制完成，或卡片 ready 时的完成并推进。确认请求携带用户看到的 `signalId`；过期 id 返回 `workflow_node_signal_changed`，不会落到新的回合上。attempt 失败、节点完成和运行取消都会撤销对应的 token；Desktop 重启后完成工具随持久化会话一起恢复。

自动重试的最后一次尝试没有产生可用结果时，步骤显示「节点运行失败」。普通完成与自动完成都会被拦住，后继保持未启动。点击「重试本步」可在原会话继续，恢复后仍保留先前的失败记录。空重试的警告为 `retry_without_result`，失败记录保存在 `payload.promptFailures`。确认强制完成时覆盖原因写入 `completion_forced.retry_reason`。停靠、普通完成、自动接受与强制完成均采用最终尝试的同一产出窗口。

节点可以在 `agentConfig.completion.files` 声明 1–32 个 workspace 相对的产出物路径或受限 glob：支持 `*`（不跨 `/`）、`**`（跨目录）和 `?`，不支持字符类、花括号、转义与取反；匹配大小写敏感、不做 Unicode 归一化，目录、gitlink 和逃出 workspace 的符号链接都不算匹配。声明 `completion` 的图必须写 `agentConfig.schemaVersion: 4`；版本 3 携带 `completion`、版本大于 4、或出现 `criteria`、`checklist` 等未定义字段时，图校验直接拒绝，而不是静默丢弃。编辑器在首次添加文件时写入版本 4，移除最后一个文件时清空声明并回到版本 3。

文件检查在每次点击完成时按现场 workspace 执行，不写入 `payload.completion`：每个声明至少要有一个匹配的普通文件存在，`mustChange: true` 还要求至少一个匹配文件相对节点启动时持久化的 worktree 基线发生变化。缺失或未变更分别返回 `file_missing` / `file_unchanged`，blocker 的 `params.path` 是原始声明；补齐或修改文件后可以直接重试，不需要重新停靠；`force` 可以覆盖这些 blocker 并写入 `completion_forced` 审计。基线缺失或损坏时，存在性仍然检查，`mustChange` 无法判定而降级为 `file_check_unavailable` warning，随完成快照投影可见并在完成后保留。Git 忽略的文件或内容快照超出预算时，存在性仍按 workspace 内的实际普通文件判定；旧基线用 `null` 记录、无法证明二进制文件是否变更时，同样只产生这个 warning。

workflow 自己发起的首轮回合本身从不读作 ready：它的 `EndTurn` 只表示 agent 结束了这次回复，而这次回复可能在向用户提问。快照因此带 `turn_unconfirmed` blocker。步骤卡片提供「确认并完成」（发送 `confirmInitialTurn` 的普通完成，不写强制审计）与「继续当前步骤」，右下角推进按钮在该回合 ready 之前不可用。由用户回复开启的回合仍按上面的规则：`EndTurn` 且没有 blocker 时正常完成，卡片重新给出「完成并进入下一步」。`agent` 策略的首轮有信号时，确认必须同时带 `confirmInitialTurn` 与 `confirmAgentCompletion`，一个字段不能解除另一个 gate；首轮没有信号时，用户仍可通过二次确认路径同时发送 `confirmInitialTurn` 与 `force` 推进并写入强制审计，单独 `force` 会被 `turn_unconfirmed` 拒绝；其余同时设置两个字段的请求返回 `invalid_request`。

每次停靠都在写入处分类：`parkKind` 为 `awaiting_user` 的包括首轮未确认、等待 agent 信号或确认、用户主动停止和 provisional 快照，超时、截断、拒答、未知终态、会话崩溃、流断开与输出契约失败都是 `abnormal`；`stopCause` 记录 owning prompt 的结束原因（`user_stopped`、`prompt_timed_out`、`stream_dropped`、`session_crashed`、`unknown`）。异常停靠的卡片显示原因，只提供继续或二次确认后的强制完成；右下角按钮只对 ready 的回合启用。缺这些字段的旧快照沿用原来的「已暂停／状态未知」展示，provisional 快照被修复时保留它停靠时的 origin 与 stop cause。

如果流在 stop reason 持久化前被 drop，节点按最后一次可读历史写 provisional 快照：保留它描述的 `userSeq`，并带 `turn_not_settled` warning；之后完成时，只要同一 user 消息的回合已经收敛就修复该快照。升级前停靠、完全没有快照的节点由第一次完成尝试在读取会话历史后物化快照，再走同一套 blocker 门禁，所以未完成的回合同样会被拒绝，不会静默推进。

每个人工回合都带一个从冻结图渲染的 `<workflow_step_boundary>` 块：它写明当前步骤与直接后续节点，并声明只完成本步骤；它不承诺运行时会阻止后续节点，而记录给用户的 prompt 仍是用户原文。

`force` 完成会在节点 payload 的 `completion_forced` 下留痕，记录 `by: "human"`、它实际覆盖的有效 blockers、解释这些 blocker 的 `stop_cause`、本轮是否有终态，以及采用的输出从哪来（`turn_window` 或 `last_assistant_message`）。这条审计与完成在同一个事务里写入，所以已完成的节点不会缺这条记录。

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

| 类型                                                                                                                                          | 可恢复 | 自动重试 |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ------ | -------- |
| `session`、`session_ended_without_stop_reason`、`session_binding_rejected`                                                                    | 是     | 是       |
| `workflow_model_not_found`、`missing_agent_config`、`interrupted_by_restart`、`repository`、`baseline_persist`                                | 是     | 否       |
| `structured_output`、`agent_refusal`、`unknown_stop_reason`                                                                                   | 否     | 是       |
| `multiple_outputs`                                                                                                                            | 否     | 否       |
| `prompt_template`、`missing_agent_ref`、`missing_skill_materialization`、`invalid_run_payload`、`condition_evaluation`、`aggregator_no_match` | 否     | 否       |

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

| 方式         | 效果                                    | 不可用的情况                                                      |
| ------------ | --------------------------------------- | ----------------------------------------------------------------- |
| `keep`       | 保持 Workspace 不变                     | 从不                                                              |
| `node_files` | 只恢复失败节点改动的路径                | `no_file_changes`；恢复单位是容器时为 `composite_region`          |
| `checkpoint` | 把整个 Workspace 恢复到恢复单位的检查点 | `no_checkpoint`、`siblings_ran_after_checkpoint`、`not_resumable` |

被自动重试替代的节点与它重试链上的每次尝试（`payload.retry_chain`）作为一个单位回滚。按需的 AI 诊断写入 `payload.ai_diagnosis`，标注为推测，调度、恢复、回滚和快照切换从不读取它。

## 查询运行内调试历史

退避中的自动重试可沿重试链读取最近一次已记录的派发输入。节点、作用域、容器执行、项索引、快照和修订必须匹配。预览通过 `inputSourceNodeRunId` 标明来源尝试；最近一次已派发尝试有任一条件不符即拒绝预览，即使更早的派发符合条件。没有派发证据时也拒绝预览。

`workflowRun.debugHistory({ runId })` 按受理顺序返回每次确认的请求键、预览、补丁和阶段。停稳成功并提交后，记录给出父修订和新修订；停稳失败的记录保留原因和无法证明已停稳的尝试（`unsettledNodeRunIds`）。每条记录同时保存受理时运行的状态和错误（`priorRunStatus`、`priorError`）：失败、取消或成功的运行在停稳期间保持原终态，在修订生效的同一次提交中回到运行中。重连后可以用原请求键关联同一次操作。停稳进行中，或停稳失败且栅栏尚未释放时，新的预览、新的确认、失败恢复、整次重启和快照切换都以 `workflow_debug_operation_active` 拒绝。拒绝以带类型的公开错误到达页面：`workflow_debug_stale_preview`、`workflow_debug_operation_active`、`workflow_debug_request_conflict`，带字段路径和原因的 `workflow_debug_patch_invalid`，以及带缺失事实或位置的 `workflow_debug_target_rejected`。

`workflowRun.debugAttempt({ runId, nodeRunId })` 可以读取已被替代的尝试，返回所属轮或项、当时的配置修订、结果和输入证据是否齐备。foreach 的第 3 项使用 `index=2`，while 的第 3 轮使用 `round=3`。升级前缺少关联证据的尝试返回空配置与空修订。

调试预览用 `nodeRunId` 选中已有尝试，用 `pendingTarget: { rootScopeId, scopeId, nodeId }` 选中尚未派发的位置。foreach 成员还需提供 `containerNodeRunId` 和从零开始的 `index`。预览按调度器的就绪条件核验前置和路径。若正常调度在确认前抢先派发，刷新后可选择实际尝试；修订生效后，历史记录用 `firstNodeRunId` 指向调度器为该位置首次登记的尝试，此前为空。若只有独立兄弟推进且目标及依赖仍有效，原预览可继续确认。选择容器入口时，预览会列出新容器将使用的成员配置。Iteration 的迭代源只接受声明为数组的根变量，指向嵌套数组的路径在 `iteratorSelector` 字段上被拒绝。

失败恢复和整次重启都以运行当前的有效修订作为配置基线。重启会创建新的根，顶层的修改继续生效；两个入口重建容器时都从容器的入口配置开始，绑定在旧容器执行上的成员覆盖只留在历史里。只有成功切换发布快照才会开始新的基线：新快照和新的运行修订一起提交，之前的补丁和尝试留在历史里可供查询但不再重放，切回曾用过的快照也一样，切换前取得的预览随之过期。被拒绝的切换保留原快照和已有修订。

## 重启后的恢复

确认调试后，如果在旧尝试停稳期间取消，操作回执记为 `stop_failed`，原因为 `run_cancelled`。正在执行的工作流转为取消；原本已结束的工作流保留原终态和原因。如果修订先生效，则修订保留在历史里，取消阻止剩余派发。

修订生效后，只绑定到退役尝试的会话会拒绝后续提示，并返回 `workflow_node_not_awaiting_input`。要继续对话，请打开替代尝试。

启动时，Desktop 先检查未结束的调试操作，再做普通中断清理。无法证明外部写入已隔离时，历史列出未停稳的尝试 ID，该范围继续阻塞。若修订已生效，调度器可补派已就绪但未认领的位置；新修订下已经启动的尝试按普通中断规则处理，不会再派发一次。

Desktop 只有在持有应用实例锁之后才运行启动扫尾（见 [Desktop 宿主](../desktop/desktop-host.zh.md#启动)），所以存活的第二个进程永远不会把运行中的工作标记为已中断。被中断的外层行以 `interrupted_by_restart` 失败并让运行失败；运行中的容器内被中断的行失败，容器和运行保留，该轮以失败结算。合法的交互等待被保留，副作用未知的工具从不自动重放。
