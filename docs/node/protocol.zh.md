# Controller–Node 协议

[English](protocol.md) | 中文

`ora-node-protocol`（`crates/node-protocol`）定义 Controller 和 Node 之间版本 1 的线上契约：有类型的消息和经过
校验的异步帧编解码器。传输、会话、Git 和持久执行属于它的使用方（[控制会话](control-session.zh.md)、Node 和
Controller）。这个 crate 不依赖任何领域、插件、文件系统或持久化 crate；公开 API 从 `lib.rs` 导出。根决策是
[Controller–Node 协议](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/0-controller-node-protocol.md)
ADR。

## 线上格式

```text
4 字节大端长度 | 1 字节帧类型 | JSON 信封
```

长度覆盖类型字节和 JSON，不含长度前缀本身。`MAX_FRAME_LENGTH` 为 16 MiB。每条消息都使用
`NODE_MESSAGE_FRAME_TYPE = 0x01`，由 JSON 中的 `message_type` 选择具体消息。在 WebSocket 上，一条二进制消息承载
类型字节和 JSON，不带长度前缀。

每个信封都有 `protocol_version`、`message_type` 和 `payload`，以及随消息而定的关联字段：命令带 `operation_id` 和
`execution_id`（可选 `request_id`），事件再加 `sequence`，`EventAck` 指明 `(operation_id, execution_id, sequence)`。
名称和标签使用 snake_case：

```json
{ "protocol_version": 1, "message_type": "hello", "payload": { "controller_id": "controller-1", "supported_versions": [1] } }
```

`Hello` 声明一个非空、无重复且包含信封版本的版本列表；`HelloAccepted` 选定一个版本，并声明一个非空、无重复的能力
集合。把响应与之前的 Hello 对应起来以及强制握手顺序，属于会话的职责。

## 能力

| 能力 | Controller → Node | Node → Controller |
|---|---|---|
| `worktree_execution` | `ensure_worktree`、`remove_worktree` | `worktree_ready`、`worktree_failed`、`worktree_removed`、`worktree_removal_failed` |
| `repository_clone` | `clone_repository` | `clone_result` |
| `runtime_control` | `bind_runtime` 和各个 `controlled_*` 信封 | `runtime_control_state` |
| `plugin_install` | `install_plugins`、`remove_plugins` | `plugins_result` |
| `agent_session` | `start_agent_session`、`submit_user_turn`、`end_session` | `thread_event`、`agent_session_ended`、`session_command_accepted`、`session_command_rejected` |
| `shared_checkout_sessions` | 每个 checkout 最多 8 个未结束 Agent 会话，恢复围栏不变 | 拒绝的启动以 `agent_failed{checkout_busy}` 或 `agent_failed{checkout_session_limit}` 结束，不关闭连接 |
| `model_proxy` | 绑定了模型的会话启动 | `hello_accepted` 中的 `model_adapter`（见下文） |
| `revision_delivery` | `deliver_revision`、`upload_grant` | `revision_result`、`upload_grant_needed` |
| `revision_restore` | 带 `prior_revision` 的 `start_agent_session` | `agent_session_ended` 中的恢复失败代码 |

始终可用：`hello`、`heartbeat`、`get_execution_status` / `execution_status` 和 `event_ack`。编解码器只校验声明；
会话在派发之前仍要检查选定 Node 的能力。

### 运行时控制

有了 `runtime_control`，Cloud 授权的工作会包在一个 `RuntimeBinding` 里到达：租户、Workspace、沙盒和运行时代次；
Node id、incarnation 和实例；Controller、控制 epoch 和版本；控制会话和操作的用户；operation、execution 和 Node
operation id；`input_closed`；以及签发和过期时间。`ControlledClone`、`ControlledPlugins`、
`ControlledStartAgentSession` 和 `ControlledDeliverRevision` 把一个绑定和一条命令配成一对，在受理之前就拒绝属于
其他执行、operation 或目标的许可。`RuntimeControlState` 确认输入已关闭，并附上未完成的 execution id 列表（空列表
也是观察到的事实）。见[运行时控制](../controller/runtime-control.zh.md)。

### 消息契约

- **Clone。** `clone_repository` 带 `node_id`、`repository_url`、`snapshot_ref`、`base_commit` 和 `workspace_ref`。
  仓库 URL 是不含 userinfo、query 和 fragment 的 `https://`；快照 ref 指明规范的 Binding UUID 并恰好指明基础 commit；
  Workspace ref（`refs/heads/workspaces/<workspaceId>/<branch>`）指明规范的 Workspace UUID 和一个不是 `HEAD` 的分支。
  旧的 `repository`/`branch` 形状、SSH 和未知的 spec 字段（目标位置、凭据、Git 选项）都会被拒绝。`clone_ready` 带原始
  spec、发起时的 incarnation、`RepositoryId`、Node 范围内的路径和完整的 commit id；`clone_failed` 带结构化的错误码
  （包括 `snapshot_unavailable` 和 `base_commit_mismatch`），以及 `no_directory` 或 `retained { repository_id, path }`
  残留。从不携带原始诊断信息。见 [仓库 clone](repository-clone.zh.md)。
- **插件。** 输入中每个规范的 `<namespace>/<identifier>` 只出现一次，带一个通用的 HTTP(S) 下载地址或按目标平台区分
  的多个下载地址，每个都带小写的 SHA-256。单项失败按插件报告；`plugins_failed` 表示整个执行无法运行。
- **Agent 会话。** 会话指明 Agent 插件和版本、它所用 checkout 对应的 clone 执行（从不是 Node 路径）、一个不会破坏
  签名行的 Git 身份，以及第一个用户轮次（文本，最多 64 KiB），可选 `model_binding_id` 和 `prior_revision`。
  由插件自带模型 adapter 的会话还带 `agent_configuration`：插件 id、版本和 target，包的 SHA-256，schema 和 adapter
  契约版本，声明指纹，配置修订号，以 JSON 文本保存的已校验 Settings（一个最多 256 KiB 的对象），协议，以及绑定模型的
  id、名称、上下文窗口和输出上限。它总是与 `model_binding_id` 一起出现，从不携带凭据。Node 在 `hello_accepted` 里于
  `model_proxy` 旁用 `model_adapter` 宣告它能这样运行什么（契约版本、target、协议）；Controller 只把这类会话派发给
  `model_adapter` 能接纳该冻结配置的 Node。旧 Node 不宣告，也不会收到这类会话。它不会忽略这个未知字段：消息解码失败属于协议违规，Node 会关闭整个控制会话
  （WebSocket 上是 `1002`，IPC 上直接结束流），并丢弃其中尚未接纳的排队工作，而不只是拒绝这一次会话启动（见
  [控制会话](control-session.zh.md#关闭)）。
  `thread_event` 携带一条已定稿的 `ora-history` 记录，作为最多 256 KiB 的不透明 JSON 对象（`truncated` 标记被截短的
  记录），并与终止的 `agent_session_ended` 共享该执行无间隙的序号，后者的 `detail` 是一个简短的 snake_case 代码。
  命令带 `command_id`；它们的回复没有序号，不需要确认，丢失的回复靠重发找回。
- **Revision。** 交付指明已结束的会话、clone、基础 commit、`refs/ora/revisions/` 下的一个 ref、Git Server 的
  `git_repository_url`、`refs/ora/deliveries/` 下的 `delivery_ref`、`workspace_ref` 和一个会话历史对象 key。
  `revision_delivered` 要求最终 commit 与基础 commit 不同；`revision_unchanged` 允许等于基础 commit，或在有
  `prior_revision` 时等于上一个最终 commit。会话的
  `prior_revision { revision_id, final_commit, base_commit, git_repository_url, delivery_ref }` 指明它要恢复的
  Revision；交付中的 `prior_revision` 只带 `revision_id` 和 `final_commit`。会话还带有
  [`ora-git list`](ora-git.zh.md) 显示的 Workspace 目录（`workspace_directory`）。恢复失败时，会话以 `agent_failed` 结束，带
  `prior_revision_unavailable` 或 `prior_revision_base_unavailable`。
- **授权。** `upload_grant` 和 `upload_grant_needed` 只存在于内存：没有序号，不需要确认，从不持久化或记录日志，`PresignedUrl` 在 `Debug` 中会隐去自己。
- **Worktree。** 两条命令都带一个 `WorktreeExecutionSpec`（资源身份、不透明的 `RepositoryRef`、Main Workspace 绑定、
  基础 ref、期望的分支和 `NodeManaged { directory_name }` 路径策略）。编解码器只检查值非空；Node 在使用前校验路径。
  见 [Worktree 执行](worktree-execution.zh.md)。

各结果族在无标签的 `ExecutionResult` 内使用互不相交的 `kind` 标签；Revision 族被装箱，以保持 `ExecutionState` 较小。

## Session MCP 生命周期契约类型

现用帧枚举包含 `UpdateSessionMcp`、`GetSessionMcpState`、`ProbeSessionMcp` 和 `SessionMcpReply`，
使用 `SessionMcpMessage<P>` 信封。启动时通过 `mcp_lifecycle: true` 显式启用，默认仍为 false。
支持 Agent 会话的 Node 声明 `session_mcp_lifecycle`。这些帧带 operation/execution 关联和最多 128 个 UTF-8 字节的 request id，
没有事件序号或确认。直接 serde 转换后仍须调用 `ValidateMessage`。

更新区分 configured 成员（精确版本、配置 revision 和标量 Settings）与 unavailable 成员（id 和封闭的
缺插件/设置码），同样要求 canonical ID 排序、最多 16 项及 64 KiB。generation 和 revision 不超过
`2^53 - 1`；新更新从 1 开始，手动探测可针对启动 generation 0，但 probe attempt 必须非零。输入 Debug
隐藏值；快照身份类型无法表示 Settings、命令或 header，健康与收敛使用不同的封闭错误码。current 快照
要求 desired/active 身份完全相同，不存在、未启用、已结束的会话或容量拒绝回复不能带快照。配置 revision
冲突的 code 放在回复自身，与当前 desired 状态的收敛失败分开。授权、不可变选择集合、
更新顺序与禁止持久化由接收请求的会话 owner 负责，不由这些结构校验器承担。

通过授权校验后，owner 在回复 Accepted 前发布完整的新 desired generation。选择集合不可增减；
较旧代际返回 stale，相同重传返回 duplicate，仍被持有的配置 revision 不允许换值。更新和健康观察只保存在内存。
每条连接最多保留 64 个未回答的生命周期请求，超额返回 `capacity_exceeded`，不解析包或保留输入值。

新生命周期 `SubmitUserTurn` 必须携带 `mcp_generation`，缺失或高于当前 desired 的下限返回
`mcp_update_required`，不进入持久队列；精确重传比较先于该检查。实际 prompt 准入等待最新代际，
在与更新共用的边界内提交 Executed。前序已准入轮次的业务执行时间不计入排队轮次的 MCP 预算，更新和重试不重置已累计的阻塞时间。轮次因 MCP 阻塞满 300 秒后发出有序可靠的 `SessionTurnFailed`，
错误码为 `mcp_refresh_wait_timeout`；排队命令的失败状态与 outbox 事件原子提交。会话继续存活，EndSession 优先。
见[会话生命周期与宿主探测](session-stop.zh.md)。

## 身份、状态与确认

身份序列化为不透明字符串；编解码器拒绝空值或只有空白的值，接受的值原样保留。`ControllerId` 和 `NodeId` 是持久的
对端，`NodeIncarnationId` 是一个运行中的 Node，`OperationId` 是 Controller 拥有的业务操作，`ExecutionId` 是对它的一次
尝试，`Sequence` 是事件在一个执行内的位置。未知的结果从不授权新的执行身份；编解码器既不生成身份，也不检查序号是否
单调。

`ExecutionStatus` 是 `Unknown`、`Accepted`、`Running`，或带着保留的终止结果的 `Completed`。`Unknown` 表示证据不足，
从不意味着可以重复一个副作用。外层的报告者是当前的运行时；已完成的结果保留它原来的运行时身份，两者之间只要求持久的
NodeId 相同（更新的 incarnation 可以报告更早的结果）。状态回复不带序号，从不确认事件；`EventAck` 指明确切的事件。
持久受理先于确认，未确认的事件会主动重放，与查询无关。

## 使用编解码器

| 调用方 | 发送 | 接收 |
|---|---|---|
| Controller | `write_controller_message(ControllerToNodeMessage)` | `read_node_message` → `NodeToControllerMessage` |
| Node | `write_node_message(NodeToControllerMessage)` | `read_controller_message` → `ControllerToNodeMessage` |

每次读取返回一条经过校验的消息，或在帧开始之前遇到干净的 EOF 时返回 `None`。每次写入校验、序列化、检查大小、写入并
flush 一条消息，在写出任何字节之前就拒绝无效消息；flush 成功并不意味着对方收到或执行了。在传输边界使用这些函数，
因为构造函数和直接的 serde 转换不会运行语义校验。每个流方向只有一个 owner，并把整次写入串行化；编解码器在取消时不保留
半帧的进度，出错后也无法重新同步，所以连接的 owner 丢弃这个流，让会话恢复去对账。`FrameError` 区分 `Io`、
`InvalidLength`、`UnsupportedFrameType`、`EncodeJson`、`DecodeJson` 和 `InvalidMessage`。编解码器不输出日志。

测试入口是 `crates/node-protocol/tests/protocol.rs`，每个消息族都有夹具，在两个方向上比较独立的线上 JSON 和有类型的
消息；运行 `cargo test -p ora-node-protocol`。
