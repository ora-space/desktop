# Cloud Agent 会话中继

[English](agent-relay.md) | 中文

Cloud 模式下，`ora-controller` 在 Cloud 与 Workspace 的 Node 之间转交 Agent 会话执行和命令。
IssueRun 阶段与持久回执由 Cloud 所有；Controller 只维护连接内的队列。SQLite 本地模式仍只承担 clone。

## 派发与权限

`ClaimWork` 的会话工作项指定 Workspace 沙盒和 Node。Controller 等待目标沙盒完成身份匹配的握手并声明
`agent_session` 能力，持有沙盒 admission gate 时向 Cloud 登记原始输入。重连完成会唤醒排队工作。
Node operation ID 从登记回包读取，不假设它等于 IssueRun ID。

已登记执行通过状态查询恢复。同一连接收到 `Unknown` 后最多重发一次原始命令，重新取得执行许可，发送
`ControlledStartAgentSession`。许可缺失或关闭时不能启动。quiesce 判断沙盒是否 idle 时，将未结束的
Agent 会话计入未完成责任。

## 事件与恢复

每个执行拥有独立的有序接管任务。一批最多 64 条，从第一条起最多等待 100 ms；编码载荷累计约 1 MiB 时
提前提交，避免超过 gRPC 消息上限。Cloud 确认整批后才逐条发送精确 ACK。终态 envelope 等待此前全部批次
接管成功；Completed 状态查询没有序号，不能证明此前记录已交付，因此不会直接结算会话。

确定未提交的不可用错误等待 250 ms 后重试，不 ACK。回复丢失沿用 RPC adapter 的同一 submission ID
有限重试。冲突或不确定写入重试耗尽时关闭连接、不 ACK，重连后从 Node 持久 outbox 重放。
仅清空内存队列不能触发 Node 重放，可能使已满的窗口永远停住。

Thread 接管、命令投递、普通结果协调和状态轮询均在传输收发循环之外运行。一个执行的 Thread 请求变慢，
不会阻止其他执行接管或心跳收发。连接结束取消其所属任务，不创建 Cloud 状态的本地持久副本。

## 命令

`ThreadCommandAvailable` 唤醒投递，周期轮询兜底。命令必须匹配已登记的运行、执行与当前沙盒运行权限。
每个运行只允许队首命令投递，且先等待 Node 状态确认它已知该执行。发送后，仅匹配的
`SessionCommandAccepted` 或 `SessionCommandRejected(session_ended)` 才允许调用
`RecordThreadCommandDelivered`；发送成功本身不是投递证据。重试保留原 command ID 与内容。
Controller 重启后重新领取 Cloud 中未确认的队首命令，Node 按 ID 去重。

## 验证与剩余工作

`cargo test -p ora-controller --lib --tests` 包含 `tests/workspaces/agents.rs` 中的测试：使用真实生成的
gRPC 与 WebSocket 接口，Cloud 和 Node 采用内存替身，覆盖有序接管、慢执行隔离、不可用／冲突／回复丢失
恢复、Controller 重启、命令重试／顺序／拒绝、能力门槛、大记录分批和 quiesce 责任。
这些测试不证明真实 PostgreSQL、生产 Node 或浏览器 Thread 联调完成。

Controller session ADR 仍为 `proposed`。本次提供可评审的实现行为，不代表设计已获批准，也不更新已批准
核心用例的证据。Revision 交付与上传授权中继留待后续开发。Node 进程 scope 管理，以及完整 M2 的
Compose／页面验收仍需各自的验证证据。
