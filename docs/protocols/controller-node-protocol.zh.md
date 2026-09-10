# Controller–Node Protocol

[English](controller-node-protocol.md) | 中文

`ora-node-protocol` 定义 Controller–Node 会话消息和 Worktree 执行的 version 1 wire 契约，
提供类型化消息和带校验的异步 frame codec。Transport、会话编排、Git 操作和持久执行由消费端负责。

该契约支持目标 Node 已有 Main Workspace 的 Worktree 闭环。Node 作用域内的资源引用在本地 IPC、
SSH 和网络传输中保持相同含义。crate 不依赖 `ora-domain`、`ora-plugin-protocol`、文件系统
或持久化实现；公开 API 从 [`lib.rs`](../../crates/node-protocol/src/lib.rs) 导出。

## 使用 codec

函数名表示**发送方**，读取函数也遵循这一规则：

| 调用方     | 发送                                                       | 接收                                                      |
| ---------- | ---------------------------------------------------------- | --------------------------------------------------------- |
| Controller | `write_controller_message`，传入 `ControllerToNodeMessage` | `read_node_message`，返回 `NodeToControllerMessage`       |
| Node       | `write_node_message`，传入 `NodeToControllerMessage`       | `read_controller_message`，返回 `ControllerToNodeMessage` |

公开函数接收 `AsyncRead + Unpin` 或 `AsyncWrite + Unpin` 字节流。消息类型由方向固定，
任意可序列化消息不是公开 codec 的扩展入口。Transport 边界应使用这些函数：构造函数和直接
Serde 转换不执行 codec 的语义校验。

每次读取返回一条已校验消息；新 frame 开始前遇到 EOF 返回 `None`。每次发送依次完成校验、
序列化、长度检查、写入和 flush。校验、序列化或长度拒绝发生在任何消息字节写出之前。I/O
失败可能留下部分字节；flush 成功不代表对端已收到、已执行或已持久接管。

每个流方向应由单一所有者管理，完整消息的写入必须串行化。这些操作不会跨取消保留部分 frame
进度，不能取消读写后在同一流上重新开始 framing。codec 也不提供错误后的重新同步；连接管理方
应丢弃该流，由会话恢复流程对账尚未完成的工作。

## Wire 格式与校验

```text
4 字节 big-endian 长度 | 1 字节 frame type | JSON envelope
```

长度包含 type 字节和 JSON 字节，不包含四字节长度前缀。`MAX_FRAME_LENGTH` 为 16 MiB，
因此 JSON payload 最大为 16 MiB 减一字节。全部消息使用 `NODE_MESSAGE_FRAME_TYPE = 0x01`，
业务消息由 JSON 中的 `message_type` 区分。实际编码为二进制 framing 加 JSON，没有另一套
`JsonDebug` 编码。

每个 envelope 都包含 `protocol_version`、`message_type` 和 `payload`。关联字段按消息确定：

| 方向              | 消息                                                                          | 必需关联字段                               | 可选关联字段 |
| ----------------- | ----------------------------------------------------------------------------- | ------------------------------------------ | ------------ |
| Controller → Node | `Hello`                                                                       | 无                                         | 无           |
| Controller → Node | `EnsureWorktree`、`RemoveWorktree`                                            | `operation_id`、`execution_id`             | `request_id` |
| Controller → Node | `GetExecutionStatus`                                                          | `operation_id`、`execution_id`             | 无           |
| Controller → Node | `EventAck`                                                                    | `operation_id`、`execution_id`、`sequence` | 无           |
| Node → Controller | `HelloAccepted`、`Heartbeat`                                                  | 无                                         | 无           |
| Node → Controller | `ExecutionStatus`                                                             | `operation_id`、`execution_id`             | 无           |
| Node → Controller | `WorktreeReady`、`WorktreeFailed`、`WorktreeRemoved`、`WorktreeRemovalFailed` | `operation_id`、`execution_id`、`sequence` | `request_id` |

Wire 字段名和 enum tag 使用 snake_case。例如 Hello frame 内的 JSON 为：

```json
{
  "protocol_version": 1,
  "message_type": "hello",
  "payload": {
    "controller_id": "controller-1",
    "supported_versions": [1]
  }
}
```

Codec 的收发路径均要求 envelope version 为 1。`Hello` 的版本列表非空、无重复且包含 envelope
version，也可以声明其他版本。`HelloAccepted` 选择的版本必须等于 envelope version，能力集
无重复且包含 `worktree_execution`，这是当前唯一定义的能力。这些检查保证单条消息自洽；
将回复与先前的 Hello 匹配、约束握手顺序需要会话实现。心跳携带当前 Node 身份，不是执行证据。

解码检查必需字段、已知 enum variant 和消息方向。Payload 匹配表示满足所选消息的结构要求：
创建和删除命令有意共用同一种 spec 结构。未知对象字段通常被忽略，因此解码不承诺拒绝所有
额外字段。

`FrameError` 区分 `Io`、`InvalidLength`、`UnsupportedFrameType`、`EncodeJson`、
`DecodeJson` 和 `InvalidMessage`。字段缺失、方向错误和 payload 结构不兼容属于解码错误；
结构合法但违反语义约束的消息携带 `MessageValidationError`。截断 frame 保留 I/O 错误类别，
与 clean EOF 区分。

## 身份与 Worktree 边界

身份值序列化为不透明字符串。Codec 拒绝空或纯空白身份及必需领域字符串，接受的值原样保留，
不裁剪空白或规范化。

| 身份                        | 含义                                  |
| --------------------------- | ------------------------------------- |
| `ControllerId`、`NodeId`    | 对端的持久身份                        |
| `NodeIncarnationId`         | Node 的一次运行实例                   |
| `RequestId`                 | 原逻辑 Client 命令，被透传时使用      |
| `OperationId`               | Controller 接管的业务操作             |
| `ExecutionId`               | 该操作的一次执行尝试                  |
| `WorkspaceId`、`WorktreeId` | Workspace 和受管理的 task worktree    |
| `Sequence`                  | 执行内的事件位置，序列化为 `u64` 数字 |

`OperationId` 与 `ExecutionId` 是不同身份。Worktree 契约要求重传或重启保留原身份和输入；
结果未知不授权更换执行身份。Codec 承载这些值，不生成身份、跟踪执行尝试或检查 sequence
单调性。Sequence 可以表示零，顺序约束由消费端负责。

两个 Worktree 命令都携带 `WorktreeExecutionSpec`，包含资源身份、不透明的 `RepositoryRef`、
Main Workspace 绑定、base ref、期望分支和 `NodeManaged { directory_name }` 路径策略。
Node 选择并授权 worktree 根目录。Codec 检查必需值非空白，不解析仓库、校验 Git 语法、
规范化路径或验证路径包含关系。非空白的目录名在用于文件系统操作前，仍须由 Node 校验路径安全。

创建成功返回 Node 作用域内的路径、分支和 base commit 事实。删除成功区分 `Removed` 与幂等的
`AlreadyAbsent`；失败包含稳定错误码和非空白诊断消息。消费端使用错误码作判断，并将
`NodePath` 视为目标 Node 文件系统命名空间中的值，不能当作 Controller 或 Client 的本地路径。

## 状态、历史结果与确认

`GetExecutionStatus` 查询目标 Node 上原操作和执行的状态。`ExecutionStatus` 表示
`Unknown`、`Accepted`、`Running`，或带已保留终态结果的 `Completed`。
`Unknown` 表示证据不足，不授权重复执行外部副作用。

外层 `ExecutionStatus.node` 标识当前报告者。Completed 的四种终态 variant 均保留结果原始
运行实例身份：

| 报告者          | 保留的结果      | Codec 处理                      |
| --------------- | --------------- | ------------------------------- |
| Node A / 实例 2 | Node A / 实例 1 | 接受，两份身份均保留            |
| Node A / 实例 2 | Node B / 实例 1 | 以 `CompletedNodeMismatch` 拒绝 |

这里仅要求持久 NodeId 相等。会话／Controller 还须验证报告者匹配已认证会话，且执行确实派发给
该 Node；消息内部一致性不代表报告者可信。

状态回复没有事件序号，不承担原事件的交付或确认。`EventAck` 标识被确认的精确
`(execution_id, sequence)`。
[D4 恢复契约](../../specs/decisions/node/protocol/0-controller-node-protocol.md#d4身份能力和会话恢复)
要求先持久接管再确认，且未确认事件的主动重放独立于状态查询。消费端须处理查询回复与事件的
任意到达顺序，避免重复业务副作用；确认丢失可重复确认，查询已确认执行不会重新交付已清理的事件。

Crate 提供上述契约的消息表示。会话绑定、重连、去重、副作用前持久化、确认前持久化和崩溃恢复
需要有状态的 Node、Controller 实现及各自的测试证据。

## 诊断与验证

协议日志延期至 `todo-87602f0b`，codec 当前不产生日志。实际调用点的英文 TODO 指定未来事件
和字段；计划使用 `ora_logging`，typed message 诊断采用 TRACE，尚未解码为类型值的失败使用
可获得的 frame 元数据和错误分类。参见[日志事件映射](controller-node-logging.md)。

[校验证据](controller-node-validation.md) 将协议保证映射到公开接口测试，并记录未验证边界。
使用 `cargo test -p ora-node-protocol` 和
`cargo clippy -p ora-node-protocol --all-targets -- -D warnings` 运行该 crate 的测试和 lint。
