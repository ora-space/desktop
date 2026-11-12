# Node Agent 会话

[English](agent-sessions.md) | 中文

Node 用与 Desktop 相同的 [Agent 运行时](../agents/agent-runtime.zh.md)运行 Agent 会话执行。它在某个 clone 执行留下的
checkout 中，只启动该执行指明的 Agent 插件的精确版本，把每条定稿的记录先写入会话 JSONL，再作为 Thread 事件交给 Node
账本，并按受理顺序运行会话命令。代码在 `apps/ora-node/src/session`（仅 Linux）。相关决策是
[由 Node 托管的共享 Agent 运行时](https://github.com/ora-space/specs/blob/main/decisions/node/agent-runtime/0-shared-agent-runtime-crate-hosted-by-node.md)、
[流式 Thread 事件与会话命令](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/20260928-streamed-thread-events-and-session-commands.md)
和[按运行限定的模型代理](https://github.com/ora-space/specs/blob/main/decisions/node/agent-runtime/20261010-run-scoped-model-proxy.md)。

## 部署

```json
"agent": { "deno_path": "/usr/local/bin/deno", "ready_timeout_ms": 30000 }
```

Deno 路径必须是绝对路径，超时必须为正数。有这个配置段时，握手声明 `agent_session`；没有时拒绝新会话，但启动时仍会结算
未完成的会话。设置了 `process.workload_uid` 时还需要 `agent.workload_directory`：一个已存在的绝对路径，属于 Node 身份，
其他人都不可写，且不与 Node home、clone 根目录或配置的旧字段 `process.host_directory` 重叠。它由沙盒入口程序创建，Node 从不创建。
配置了 `agent.ora_git_directory`、`git_access`，且 `process.workload_uid` 不同于 Node 有效 UID 时，每个会话还有自己的 [ora-git](ora-git.zh.md) socket，以
`ORA_GIT_SOCKET` 导出。

## 每个执行一个运行时

每个会话执行组装自己的插件生命周期和 Agent 运行时。插件安装在 Node home 的 `plugins/` 下（见
[Node 插件](plugins.zh.md)），历史保存在旁边的 `sessions/` 下，但这个生命周期只报告和启动该执行的 Agent 插件，所以
"只有指明的插件"和"身份只到达本执行的进程"在构造上就成立。

| 运行时接口 | Node 的实现 |
|---|---|
| `SessionStore` | `MemorySessionStore`：Node 重启后从不恢复会话 |
| `AgentAttach` | 该执行自己的单插件生命周期；没有 Effect 消费者（Node 不投射 Skill） |
| `SessionSetup` | `NoSessionMcp`：MCP 配置在 Node 上没有投递路径 |
| `RuntimeEvents` | 每一行历史都变成 Thread 事件；标题和模型目录事件被丢弃 |
| `WorkspaceDirectory` | checkout |

没有人在场的宿主用运行时的入口代替用户操作：`wait_for_agent` 等待 Agent 连接变为 `Ready` 或它的监督者放弃，调用方不设
上限时就一直等；`start_session_with_id` 用持久的执行身份命名会话（已被使用的身份会被拒绝）；
`prompt_session_as_message` 把宿主的消息身份记为用户消息中每个块的 ACP `messageId`。

运行的 Git 身份以 `GIT_AUTHOR_*`/`GIT_COMMITTER_*` 导出到插件进程以及宿主为它启动的每个进程上；Node 不写任何 Git 配置。
带冻结 Agent 配置的会话里，这些变量和会话自带的其他变量加在一个只保留 `PATH`、`LANG`、`LC_ALL` 和 `TMPDIR` 的空环境上，Node 环境中的
模型凭据、代理设置和 CLI 配置都不会到达 Agent（见 [Agent 配置](../plugins/agent-configuration.zh.md)）。其他会话仍叠加在
继承的环境之上，Node 自己的模型凭据和代理设置会到达 Agent。

### 单独的工作负载用户

设置了 `process.workload_uid` 时，Deno 插件以及它请求宿主启动的所有进程都会降到工作负载 UID（组与它相同，没有附加组，
没有 capability，`no_new_privs`，umask `077`）；无法降权的启动会失败。每个会话得到
`<workload_directory>/<sha256(execution_id)>/`（属于 root，`0711`），其中包含：

- `package/`：在新建的 `0755` 目录中以硬链接呈现已安装的包（跨文件系统时以 `0644`/`0755` 复制；链接和特殊文件会让这个
  视图被拒绝）。插件从这个视图启动，生命周期仍然校验已安装的包。
- `home/`：`0700`，属于工作负载用户。对插件和宿主启动的进程，`HOME`、XDG 目录、`DENO_DIR` 和
  `DENO_NO_UPDATE_CHECK=1` 都指向这里。

启动之前，checkout 在不跟随链接的情况下交给工作负载用户。插件停止后会话目录被删除，服务打开时也会删除遗留的目录。

## 执行如何运行

1. `start` 立即返回；会话在后台解析 checkout，找不到时以 `agent_failed{checkout_unavailable}` 结束。
2. 查找版本之前先获取插件的使用租约。版本不存在，或者包不是该版本、不是 Agent，会以
   `agent_failed{agent_plugin_unavailable}` 结束，不启动任何进程。租约一直持有到插件的整个进程树退出。
3. 有 `prior_revision` 时，会话在把 checkout 交给工作负载用户之前、在任何插件启动之前，先把它恢复到 checkout 中，见
   [Revision](revisions.zh.md#恢复上一个-revision)。
4. 有界地等待 Agent 连接（超时则 `agent_failed{agent_unavailable}`），以执行 id 作为 Ora Session id 创建会话（失败则
   `agent_failed{agent_start_failed}`），然后发送 `initial_turn`。
5. 一个轮次中定稿的每一行历史都变成带该轮次 `turn_id` 的 Thread 事件；用户消息以同一个身份作为它的 ACP `messageId`。
   超过 256 KiB 的记录在 Thread 中只保留 `at`、`seq` 和 `type` 并标记为 `truncated`，JSONL 中保留原样。
6. `command_arrived` 唤醒会话读取整个队列。`SubmitUserTurn` 在当前轮次之后按受理顺序运行，开始时结算为 `executed`。
   `EndSession` 把在它之前受理的轮次结算为 `discarded`，停止会话（取消正在运行的轮次并记录 `TurnEnded{cancelled}`），
   然后把自己结算为 `executed`。
7. 结束时，会话停止，释放运行时，停止插件并等待进程树退出，释放租约，丢弃排队的命令，写入终止结果，然后忘掉这个会话。

只有 ACP 成功完成才允许下一个轮次。prompt 出错或流意外关闭时，会话以 `agent_failed` 结束，带 `agent_turn_failed` 或
`agent_stream_closed`；原始的服务商诊断信息从不进入历史或日志，已经排队的 `EndSession` 保留它显式的原因。

## 定时运行的业务事实

Cloud 可以把会话输入作为定时 occurrence 受理（`spec.trigger = scheduled`），手工 Issue 运行则不设置该字段。只有这类会话、
且只有 Cloud 按请求结束它时（`user_ended` 或 `idle_timeout`），并且 Agent 已经完成一个轮次，才在终止结果里上报业务事实；
取消、失败或中断的会话、所有手工运行，以及最后一轮被取消或始终没有收尾的运行，都不上报。结束原因本身不证明轮次完成：
结束命令会取消仍在执行的轮次。只用工具完成了工作的轮次没有 verdict 文本，它写出的产物照常上报。

- verdict 从定稿历史的最后一个 Assistant 轮解析，且只有 `TurnEnded` 以 `end_turn` 收尾时才算数：
  `QA_VERDICT: <pass|warn|fail|blocked|idle|range_unresolved>`，可紧随一行 `QA_DETAIL: ...`。取第一个标记，大小写不敏感；
  非法值、超过 2000 字符的 detail 或没有标记都只表示「未报告」。wire 会拒绝的 detail（例如含 NUL）同样按「未报告」处理，
  所以畸形的注解不会让结算该运行的终态事务失败。不会改用更早的轮次。Node 不替 Cloud 解析 Thread，只上报类型化的值
  （`scheduled_result`），由 Cloud 校验。
- 产物在 Workspace 可能被释放之前，从 checkout 固定的 `.ora-scheduled-artifacts/` 目录只读采集。采集沿用与本地定时运行
  一致的类型白名单（markdown、json、txt、log、xml、html）、路径与符号链接／硬链接拒绝规则，以及单文件 10 MiB、单运行
  64 个、总计 64 MiB 的上限。名称按 NFC 归一后做 Unicode 大小写折叠，大小写不敏感文件系统会当成同一路径的两种拼写只声明
  一个，另一个记为拒绝。被接受的候选连同大小和 SHA-256 组成上报的 `artifact_manifest`，由 Cloud 在接收上传时核对；
  被拒绝的候选连同原因保留在清单里，不会消失。遍历的 512 条目预算在读取每个目录时就地消耗，塞满名称的目录不会被完整
  枚举、保留或排序；清单最多列出 64 条拒绝，其余拒绝计入 `omitted_rejections`，而在没有读完所有条目就停止时设置
  `walk_truncated`，所以没人检查过的候选不会被当成一份完整的拒绝清单。目录不存在就不上报任何产物，对象键最终由 Cloud
  决定；Node 不写 Workspace，也不持有对象存储凭据。已声明文件的上传是独立的
  `deliver_scheduled_artifacts` 执行，见 [Revisions](revisions.zh.md#定时产物上传)。

## 实时预览

实时预览通过 Hello／HelloAccepted 的可选 `thread_previews` 双方协商。运行时最多每 100 ms 通知一次尚未定型的正文、
思考与工具快照。Node 只保留每个 execution 的最新版本，每批最多 32 个块、256 KiB，总共最多 64 个会话。块使用分配时
就确定的历史位置，定型 `ThreadEvent.history_sequence` 覆盖同位置预览。预览不进入 SQLite、没有事件序号和 Ack，慢消费者
或断线不会终止 Agent；重连从当前完整快照恢复。缺少协商的对端继续只发送持久记录。存储能力选择见
[Controller](../controller/controller.zh.md#协调存储)。

## 记录顺序与崩溃

定型的 Agent 正文在 Node 协议中另外带一份可选纯文本投影，供后续任务交接使用。Node 从类型化的
`AgentMessageChunk` 提取投影；思考、工具与非文本块不产生投影。单条最多 64 KiB，在 UTF-8 边界截断并设置显式截断标记。
原始 JSONL 和 Thread 记录保持完整且不改写。使用方必须把缺少投影视为不可用的历史，而不能从工具或思考记录猜测回复。

运行时在一行写入文件之后、写下一行之前同步调用 `record_settled`，Node 就在这里追加 Thread 事件，所以每条 Thread 记录都
以同样的顺序存在于 JSONL 中，崩溃时 JSONL 最多领先一行。一旦 Thread 追加失败，镜像就永久停止，会话以
`agent_failed{thread_unavailable}` 结束。

重启后 Agent 从不恢复。未结束的恢复先交接原 Git 尝试，只有停止确认后才报告 `interrupted`；停止未知时会话保留未终态、禁止交付与 checkout 复用（见[Revision 恢复](revisions.zh.md)）。正常关闭会取消进行中的对话，
等待插件清理，并在释放数据库租约之前提交终止证据。会话仍在运行时，`sealed_history` 返回 `history_unavailable`，否则返回
它的 JSONL。

插件进程和 Agent CLI 由 Node 直接启动（进程组，整棵树终止），不向 Node 的 `ora-reaper` 登记。Node 崩溃时，插件会因
stdio 关闭而退出，但忽略这一点的后代进程不会被回收。

## 账本

会话账本在 `ora-node-db` 中（schema v8）；`apps/ora-node/src/session/ledger.rs` 在 `SessionJournal` 之上实现
`SessionLedger` 和 `CheckoutResolver` 两个端口。

- `NodeDatabase::session_journal` 打开一个共享 Node 排他租约的窄连接；写入使用 immediate 事务和 `FULL` 持久性，最后一个
  journal 让租约保持有效。
- 受理保存完整的不可变输入。受控受理记录运行时许可并尊重未完成的责任；启动时重新检查授权，只把 `Accepted` 推进到
  `Running`。
- Thread 追加要求处于 `Running`，并在同一事务中分配下一个序号。精确的 ACK 只删除指明的事件；序号从不重置。
- 命令保留它的信封、唯一 id 和受理顺序。完全相同的重试从不让已结算的命令重新排队，改变的输入会失败；结算是单向的。
- 结束会话时，结果、最后一个事件以及对每个排队命令的处置一起提交。之后的命令得到
  `SessionCommandRejected(session_ended)`。
- 查找 checkout 只对已成功完成的 clone 返回存储的目标路径，其他情况一律失败；不从 id 重建任何东西。

Cloud 控制的启动使用 `ControlledStartAgentSession`，要求 `runtime_control` 和 `agent_session`；裸启动只限于未设栅栏的
本地 IPC。一个连接对每个执行最多预留 256 个事件槽位，并从自己最后预留的序号之后分页，所以满了的执行从不阻塞其他执行或
控制回复；只有持久的精确 ACK 才释放槽位，重连会从最小的未确认序号开始重放。发送窗口满时记录仍在继续。

## 模型访问

`AgentSessionSpec.model_binding_id` 是可选的不透明 Cloud 引用。部署添加 `agent.model_proxy`，带绝对路径的
`gateway_url`、`ca_cert`、`client_cert` 和 `client_key`；这个客户端证书为运行时的租户、Workspace 和代次签发，与 Node 的
WSS 服务端证书分开。服务在声明 `model_proxy` 之前先校验它。

- Node 只带绑定 id 和执行 id 请求 `POST /internal/v1/model-grants`。平台返回冻结的协议和模型，以及一个只在内存中使用的
  临时 token，从不返回上游 key。
- OpenCode 使用一个 `ora-model` provider，搭配 OpenAI 兼容或 Anthropic SDK，通过环境变量占位符从
  `ORA_MODEL_ACCESS_TOKEN` 读取 token，并通过 `NODE_EXTRA_CA_CERTS` 信任平台 CA。有工作负载用户时，HOME、XDG 目录和
  `OPENCODE_CONFIG_DIR` 使用会话的 home，那里只放公开的 CA；否则使用 `model-runtime/` 下会话私有的目录。两者都在 checkout
  和 Revision 之外，token 从不写入磁盘。
- 绑定模型的 OpenCode 设置 `OPENCODE_DISABLE_PROJECT_CONFIG=true`，所以 checkout 中的 `.opencode` 配置被忽略，也没有
  东西会进入 Revision。
- 续期延长授权但不更换 token；被拒绝时取消对话。正常终止会停止续期、撤销授权并删除临时状态，然后才记录结果。经过校验的
  `model_session_ending` 拒绝最多等待 30 秒，等持久的 `EndSession` 到来。崩溃的 Node 从不恢复：Cloud 的恢复和授权过期会
  撤销它的权限。
