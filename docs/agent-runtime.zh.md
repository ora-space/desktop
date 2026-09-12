# ACP Agent 运行时

[English](agent-runtime.md) | 中文

Backend 启动时，会为每个已安装的 [Agent 插件](../crates/backend/src/agent_runtime/plugin_agent/README.md)建立一条独立监管的 ACP 连接。每个 Ora Session 拥有一个串行 actor；指向同一 Agent 的 actor 共享应用级连接，并通过私有 provider session id 路由事件。同一 Session 同时只能有一个 prompt owner，但 load 可以跟随正在进行的 prompt，不同 Session 则可以并发运行。取消只作用于目标 Session 的 prompt owner，不会把 load follower 变成 owner，也不会卸载可复用 Session。

## 进程与 Session 生命周期

- Session 合约使用规范的 `<namespace>/<name>` `agent_ref` 标识提供 Agent 的插件。namespace 是身份的一部分；运行时不认识的开放字符串表示对应 provider 当前未安装，返回 `agent_runtime_unavailable`，而不是把数据视为损坏。Ora 本身不内置 Agent。
- supervisor 以完整插件 id 为键，安装集合变化时实时协调。只有 `agent` 类型插件受监管；插件生命周期负责启动和停止进程，运行时通过 `PluginApi::attach_agent` 附着到进程，并从无损通知 tap 读取该 generation 的 `agent/acp` 帧。
- 插件可以使用包内 CLI，也可以回退到 PATH 中的用户安装。主机负责解析包内可执行文件，`package_command_missing` 明确表示包内命令不存在。机器上缺少 CLI 返回可重试的 `agent_not_installed`；无法使用的包内程序返回 `agent_unusable`，并停止本进程生命周期内的无意义重试。
- 运行时错误在产生处映射成类型化公开错误。进程启动失败为 `agent_start_failed`，运行时 deadline 为 `agent_timed_out`，模型发现失败为 `agent_model_discovery_failed`。队列溢出或 ACP 违规等用户无法处理的问题仍为 `internal_error`，并保留 request id 供诊断。
- Task worktree 通过 Task → Workspace id → 已保存 branch → Git worktree 元数据解析。既有 worktree 的路径不从创建根目录或当前进程 cwd 推断。
- Backend 启动时把遗留 Running 行恢复为 Stopped，然后每个 Agent 在独立运行时线程中完成启动和 `initialize`。各 Agent 以有上限的指数退避独立重试；一分钟内超过三次真实失败会打开熔断，本进程内不再自动重试。某个 Agent 不可用不会影响其他 Agent 或 Ora 外壳。
- `startSession` 是唯一创建入口：打开 setup 注册窗口，使用当前 Session MCP Snapshot 调用 `session/new`，应用可选模型意图，持久化 Ora Session，打开 history，并在返回前安装 actor。provider session 在持久化和 actor 注册完成前受 guard 保护；中途失败会使用 `session/delete` 或 `session/close` 释放未暴露资源。
- Load 只读取 Ora 自己的记录，不接触 Agent、创建 provider session 或改变生命周期。没有 history 文件表示空会话；无法读取的文件会显式降级。若 actor 已存在，由它从当前 durable cutoff 回放；prompt 进行中时，load 从挂接点继续跟随后续事件，但不拥有该 prompt。
- Prompt 才会获取 provider session。尚未附着的 Session 先注册当前 connection generation 的 route，调用 `session/load` 并丢弃 Agent 的回放，再标记 Running。握手产生的配置和命令会发送给客户端但不写 history。attach 和 MCP 校验失败时，发送本身失败，不会产生一个空 turn。
- 不支持或无法完成 `session/load` 的 Agent 不会终结会话。Ora 会用 `session/new` 建立替代 provider session，并在下一个 prompt 中像切换 Agent 一样注入已记录 transcript。新 binding 只有在携带 transcript 的 prompt 被接受后才持久化，从而让崩溃后的下一次 prompt 能安全重建并重新交付。

## Session MCP

已配置的 MCP 插件属于 Session Runtime Input。`startSession`、prompt attach、重建、在线刷新、workflow 启动和 Agent 替换都会解析一个 Snapshot，并作为 ACP `mcpServers` 发送。在线 Session 在内存中维护 Desired/Active revision；配置变化立即唤醒空闲 Session，并在繁忙 Session 的当前 prompt 结束后刷新。MCP 刷新与 Skill Effect 修改共享同一个 Agent Session Barrier。设置值不会进入 Effect 状态、SQLite、Workspace 文件、日志或 UI。详见 [Session MCP](session-mcp.md)。

- 连接丢失会失败该 Agent 的进行中操作，只把已注册 Session 标为 Stopped，先让插件生命周期停止旧进程，再启动 replacement。Session 仅按需重新 load，prompt 永不自动重放。
- `initialize` 会声明 session config-option 能力。模型选择依赖这一能力；Ora 当前不声明布尔配置选项，因为 UI 只渲染带 id 的 selector。

### 首个 Session 标题

新附着的 Session 在运行时拥有一个标题窗口，可立即接受合法的 `session_info_update`。只有第一个 prompt 以 `EndTurn`、`MaxTokens` 或 `MaxTurnRequests` 结束后才启动 fallback 窗口；拒绝、取消、请求失败和连接失败不消耗资格。

- 若 Agent 声明 `session/list`，actor 会在符合条件的 prompt 后第 3 秒和第 10 秒尝试获取标题；仅在空闲时请求，每次 deadline 为 5 秒。
- 若不支持 `session/list`，不会发送 list 请求，但 push 标题仍可在 10 秒窗口内生效。push 和 list 共用 `SessionTitle` 校验及持久化路径；空值、超长值和重复值会被忽略。
- scheduler 回调只入队 actor command。prompt、load、stop、switch、delete、用户重命名、队列失败、连接丢失或 actor 终止会关闭或抢占窗口。用户重命名会锁定窗口，防止晚到的 Agent 标题覆盖用户选择。
- 标题成功写入后发布 `AppEvent::SessionTitleUpdated { session_id }` 作为失效提示，前端再读取权威 `Session.title`。

actor scheduler 只保留弱 command sender，因此 manager 关闭、删除或切换释放最后一个外部 sender 后，actor 可一并释放连接、recorder、repository 和 scheduler。

## 延迟创建 Session 与模型发现

打开或切换聊天界面不会创建后端 Session。前端先在本地创建 optimistic 首个 turn，`startSession` 完成握手和持久化后再接管返回的 Ora session id，并发送 prompt。Workflow node 使用同一路径，但在 node-run binding 提交前保持未发布状态。

Session 创建前的模型来自按需调用的 `agent/list_models`，输入为 Workspace 的真实 cwd。Ora 不缓存，也不会在共享 ACP 连接启动时读取。模型发现拥有独立 60 秒预算，失败只影响该请求。Session 创建前选择的模型是本地 intent，只有新 Session 握手确实报告相同值时才应用；已存在 Session 始终使用自身配置。

## Session History

Ora 在 sessions root 下为每个 Session 保存一份 append-only JSONL。这让会话可以脱离原 provider 回放并交给其他 Agent。文件格式、顺序和失败语义由 [`ora-history`](../crates/history/README.md) 所有；运行时只决定何时写入和回放。

- Prompt 在调用 Agent 前从请求 block 写入；provider 回显的 `user_message_chunk` 被忽略，因此注入上下文不会再次进入记录。
- streamed update 先记录再转发。客户端中途断开只会丢失 stream，不会丢失 Agent 已产生的记录。
- 每个 prompt 用 `TurnEnded.stopReason` 封口，使 cancelled 与 completed 可区分，并让读端知道未完成工具的 turn 结果。
- 工具计时遵循 ACP 生命周期。第一个非终态 `pending` 或 `in_progress` 确定 `startedAt`；重复和 partial update 不重置；`completed`、`failed` 或 turn 边界使用单调时钟冻结 `durationMs`。若首条可见记录已是终态，Ora 不知道真实开始时间，因此不生成 timing。
- timing 仅随压缩后的最终工具快照持久化。turn duration 只根据用户消息和匹配 `TurnEnded` 的真实 `recordedAt` 重建；任一端缺失或非法时不推断。
- live 前端从 optimistic prompt 创建时间开始，在 operation 结束时冻结 turn duration。后端只发送时间锚点与最终 duration；renderer 使用一个共享的本地 1 秒 ticker 刷新可见标签，不轮询后端，也不产生每秒协议流量。页面切换不会丢失存放在 conversation store 中的时间，不同 Session 不能重置彼此计时。
- Agent 正常结束 turn 时，可把仍未报告终态的工具推断为 completed；其他结束原因保留未完成状态。断连、prompt 失败、队列溢出和客户端断开都以 cancelled 记录，因此不会虚构工具成功。
- 有序 session event 使 response 成为 turn fence。取消会在 grace 期内继续消费直到匹配 response；若 provider 不收敛，actor 记录已接受快照后隔离 route。
- 写入按 settled item 批量 flush 但不 fsync。完整但无法解析的 JSONL 行会作为 `unreadable_records` 提示，既有 `Gap` 会成为 `unrecorded_content`，而不是静默显示不完整会话。

### 切换 Agent

`switchSessionAgent` 保留 Session id、Task 和 history，只替换 binding。

- incoming provider 先握手并应用配置；成功前不拆旧 binding。切换到当前 Agent 返回 `session_agent_unchanged`，history 已降级返回 `session_history_degraded`。
- 切换选择在下一条消息提交；旧 binding 随后释放。transcript 延迟到下一次 prompt 才作为首个 content block 注入，放弃的切换不产生成本。
- 是否仍欠 handoff 由记录中没有后续 `HandoffDelivered` 的 `AgentSwitched` 推导，因此可跨 actor 丢失和进程重启恢复。只有 provider 接受携带 transcript 的 prompt 后才同时在内存和文件中结清。
- ACP 无法直接安装完整会话，因此每个已记录 turn 会折叠为一条明确标注为历史展示的 user message。transcript 没有大小上限，超过接收模型上下文时由 provider 报错。

### History 降级

跳过记录比停止记录更危险，因此写入失败会永久停止该 Session 的后续记录，直到显式恢复。

- 已在 streaming 的 turn 会完成；自身 prompt 无法写入的 turn 会在调用 Agent 前被拒绝。
- Session 进入带操作系统原因的 `historyState: degraded`，后续 prompt 返回 `session_history_degraded`。
- `resumeSessionHistory` 先追加说明丢失范围的 `Gap`，再恢复可写；它不会恢复已经丢失的内容。
- 完全无法读取的 history 同样降级。load 不会用空会话掩盖读取失败。

### 删除

删除 Session 会删除 Ora 自己的 history 文件；Task 和 Project cascade 会删除其所有 session 文件。Session id 在数据库 cascade 前收集。文件删除是 best effort，因为数据库记录已经不可达，而失败不应让用户看到无法删除的对象。

## 流量控制

ACP stdout 是带 8 MiB frame 上限的换行分隔 JSON-RPC。connection reader 通过无界有序通道交给持续运行的 central router；每个已注册 Session 拥有独立的 256 项 FIFO，存放 update、permission request 和终止 response。连接丢失和队列溢出走独立 control queue。因此单个 Session 的背压不会阻塞共享连接，溢出也只停止目标 Session。

setup 阶段另有最多 256 条通知的临时 buffer，只在 `session/new` 尚未返回 provider id 时存在。注册 route 时只取匹配 provider id 的通知，最后一个 setup 窗口关闭后丢弃未匹配的陈旧通知。未知 Agent JSON-RPC request 返回关联的 `-32601`；畸形帧、无法匹配的 response、超大帧和 stdio 丢失会终止连接。route 绑定 generation，旧连接或已卸载 Session 的 update 会作为 stale 丢弃。

permission request 与 update 共用有序 Session FIFO。发生在 load 或 idle 期间的 permission request 会以 cancelled 回答并报告生命周期违规。丢弃 Web body、关闭 Tauri stream 或 abort 前端 `AsyncIterable` 都发送 `session/cancel`。prompt inactivity timeout 同样先 cancel，等待 5 秒 settlement grace，再只卸载该 Session 的 route；共享连接与其他 Session 保持运行。显式 Stop 可在 Agent 支持时调用 `session/close`，并保留 provider history 供以后 load。

history replay 是唯一主动施加背压而非快速失败的 stream，因为完整历史远大于 256 项队列，而暂未消费不等于断开。

## 超时与限制

| 边界                                  | 值                                  |
| ------------------------------------- | ----------------------------------- |
| `initialize` 握手                     | 15 秒                               |
| 插件模型发现                          | 60 秒                               |
| Session setup/load inactivity         | 30 秒，每个 session update 重置     |
| Prompt meaningful-activity inactivity | 1 分钟；工具运行和权限等待期间暂停  |
| 取消收敛 grace                        | 5 秒                                |
| 连接重试退避                          | 250 ms 起，倍增至 30 秒上限         |
| 连接失败熔断                          | 1 分钟内超过 3 次失败               |
| Session 标题 list 请求                | 每次 5 秒                           |
| 首标题 fallback                       | 首个符合条件 prompt 后 3 秒和 10 秒 |
| Session update/event 队列             | 256 项                              |
| JSON-RPC frame                        | 8 MiB                               |
| 序列化 structured prompt              | 16 MiB                              |
| handoff transcript                    | 无上限                              |

Prompt deadline 是 inactivity timer，不是总预算。Agent message、thought、plan 和 tool lifecycle update 会证明 prompt 前进并重置窗口。`available_commands_update`、`current_mode_update`、`config_option_update`、`session_info_update`、`usage_update` 属于 session chrome：即使 prompt 卡住也可能继续出现，因此不刷新 deadline。

第一次观察到 pending 会重置一次；只要任意工具为 `in_progress` 就暂停，最后一个并行工具结束后重新获得完整窗口；等待权限期间同样暂停，权限返回后重新计时。因此合法长工具可以运行数小时而不超时，静默一分钟的 prompt 只失败自身 Session。系统不设 prompt 绝对运行上限。

Prompt 以有序 ACP `ContentBlock` 传递，包括文本、图片、音频、resource link 和 embedded resource。空列表、纯空白文本会被拒绝，16 MiB 限制在发送到 provider 前按序列化 JSON 计算。

## 所有权边界

Ora 删除自身数据库记录和 session history，并异步注册 Git 清理任务以移除 task worktree 与 `ora/*` branch；provider history 不随 Ora 侧删除。`session/delete` 只用于回滚从未暴露的 provider session，用户可见 Session 始终 close 而非 delete。Session 删除与 actor 新操作串行，先卸载 route，再软删除数据库行并移除 history 文件。

Ora 拥有 transcript，Agent 拥有模型上下文。transcript 可跨 Agent 携带，模型上下文不可，这也是切换时采用注入而非 provider replay 的原因。最后一个 Backend owner 释放时，所有 supervisor 停止接收新工作，取消 routed operation，并有界终止 CLI 进程树。

## 开放 Agent 身份的兼容注意事项

Agent 身份是由安装插件提供的开放字符串，目前必须是完整 canonical plugin id，例如 `official/ora-space.claude`。旧 Session 或 workflow graph 若仍保存较短拼写，会因为匹配不到 supervisor 而报告 provider unavailable，直到用户重新选择 Agent；运行时不会隐式改写数据库或 snapshot JSON。

前端 Agent catalog 完全来自运行时 `listInstalled(kind: "agent")` 与 `getAgentRuntimeStatus`。只展示 `Ready` 或 `Starting` 的 Agent；`Unavailable` 会低频轮询，`Failing` 因熔断已打开而不轮询。已保存但当前不可达的偏好不会被静默丢弃，既有 Session 的 binding 也始终按原值报告。首次运行只采用第一次探测到的 Agent，后续安装或重启不会暗中切换用户选择。
