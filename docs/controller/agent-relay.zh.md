# Cloud Agent 会话中继

[English](agent-relay.md) | 中文

Cloud 模式下，`ora-controller` 在 Cloud 与 Workspace 的 Node 之间转交 Agent 会话执行、命令、Revision
交付及其会话历史的上传授权。IssueRun 阶段与持久回执由 Cloud 所有；Controller 只维护连接内的队列。它原样转交
Cloud 在每份输入中固定的 Git Server 仓库地址与 ref，不持有、不申请、不转交任何 Git 访问凭据：Node 用自己的
git-access 身份访问 Git Server
（[ADR](../../specs/decisions/cloud/controller-integration/20261016-snapshot-clone-input-and-execution-ref-grants.md)）。
SQLite 本地模式仍只承担 clone。

## 派发与权限

`ClaimWork` 的会话工作项指定 Workspace 沙盒和 Node。会话输入中的 Workspace 目录（`workspace_directory`，
供 `ora-git list` 使用）原样到达 Node，非 run Workspace 为空的 run 专属字段省略。Controller 等待目标沙盒完成身份匹配的握手并声明
`agent_session` 能力，持有沙盒 admission gate 时向 Cloud 登记原始输入。重连完成会唤醒排队工作。
Node operation ID 从登记回包读取，不假设它等于 IssueRun ID。

已登记执行通过状态查询恢复。同一连接收到 `Unknown` 后最多重发一次原始命令，重新取得执行许可，发送
`ControlledStartAgentSession`。许可缺失或关闭时不能启动。quiesce 判断沙盒是否 idle 时，将未结束的
Agent 会话计入未完成责任。

## Revision 交付

`DeliverRevision` 工作项沿用同一领取—登记—派发路径。登记要求目标 Node 完成身份匹配的握手并声明
`revision_delivery` 能力；未声明的 Node 让工作项留在 Cloud 队列中。登记输入在重建时补上目标 Node
（Cloud 的 spec 不含 Node），Git Server 仓库地址、`delivery_ref` 与 `workspace_ref` 原样保留，并且只在取得新的
执行许可后以 `ControlledDeliverRevision` 发送。每类执行
有独立的待处理列表，交付记录不会进入 clone 轮询或 clone 协调；未结束的交付使 quiesce 不报告 idle。

`TakeOverNodeEvent` 之前，Controller 按派发输入核对终态 `RevisionResult`：`base_commit`、`revision_ref` 与
`history.key` 必须相同，unchanged 的最终提交必须等于基础提交或输入中前序的最终提交，delivered 的最终提交二者
都不等。核对不通过的结果既不接管也不 ACK。交付 ref 是否指向最终提交只由 Cloud 经 ref 查询核验，Controller
不访问 Git Server。结果在 Cloud 提交后才 ACK；Cloud 在该接管内校验声明的历史对象。Completed 状态查询不结算交付：Cloud 只接受附带 Node 有序回执的交付结果，Node 会持续重放终态
envelope 直到收到 ACK。Controller 自己从不写 `RevisionFailed`，也不报告只由 Cloud 记录的
`VERIFICATION_FAILED`。停机、租约丢失或连接断开都让已登记交付保持无结果；新进程通过状态查询以及
`Unknown` 时重发原始命令继续它。

## 上传授权

交付只上传会话历史（`history_key` 处的 JSONL），提交推送到 Git Server。`GrantRevisionUpload` 只为该键、
且只携带 Node 固定的 SHA-256 摘要申请，Cloud 因此把 `x-amz-checksum-sha256` 与 `If-None-Match: *` 一起签入。
含其他键的请求不会转给 Cloud，也得不到授权，但不让交付失败：

- Node 发送 `UploadGrantNeeded` 时，原样转交其 checksum map。
- 新连接首次看到交付处于 `Accepted` 或 `Running` 时，用该 Node 最近向本进程报告的摘要再次申请，覆盖
  重连场景。
- 派发后不立即申请：此时 Node 尚未固定对象，授权无法绑定摘要，而 Node 需要时会立即请求。同理，
  重启后的 Controller 不知道摘要，等待 Node 的请求。

返回的授权以 `UploadGrant` 转交，头与过期时间原样保留，且只接受申请过的键。Cloud 不可用或租约丢失时
保留请求稍后重试；`CONFLICT`（交付已有结果或运行不再处于交付阶段）丢弃请求，不让交付或连接失败。
授权只在转交途中存在于内存，不持久化、不写日志，消息帧也不会被格式化进诊断信息。进程内存只保留摘要，
摘要不是凭据。

## 续接会话

输入带 `prior_revision` 的会话续接该 Issue 最近的 Revision（恢复契约
`cloud/controller-integration/20261010-revision-restore-contract`）。前序 Revision 映射进 Node 会话 spec 时带
Git Server 仓库地址、交付 ref 与基础提交（仓库地址或 ref 被 Node 协议拒绝的输入被拒绝），映射进交付 spec 时只带
身份与最终提交（交付输入带仓库地址、交付 ref 或基础提交则拒绝）。Node 自己从 Git Server 拉取前序最终提交，没有
下载授权。这样的会话只在目标 Node 当前握手同时声明 `revision_restore` 时登记；否则与其他能力缺失一样留在 Cloud
队列中，不会在不能恢复的 Node 上以全新 clone 运行。

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

`cargo test -p ora-controller --lib --tests` 包含 `tests/workspaces/agents.rs`、
`tests/workspaces/deliveries.rs` 与 `tests/workspaces/restores.rs` 中的测试：使用真实生成的 gRPC 与 WebSocket 接口，Cloud 和 Node 采用
内存替身，覆盖有序接管、慢执行隔离、不可用／冲突／回复丢失恢复、Controller 重启、命令重试／顺序／拒绝、
能力门槛、大记录分批、quiesce 责任、受许可约束的交付派发、头原样保留的摘要绑定授权转交、重连刷新授权、
授权被拒、只为历史键申请授权、提交后 ACK、续接会话的 `revision_restore` 门槛、前序 Revision 与 Workspace
目录原样到达 Node，以及日志中不出现授权 URL 与签名。这些测试不证明真实 PostgreSQL、对象存储、
生产 Node 或浏览器 Thread 联调完成。

Controller session ADR 仍为 `proposed`。本次提供可评审的实现行为，不代表设计已获批准，也不更新已批准
核心用例的证据。Node 进程 scope 管理、生产 Node 交付，以及完整 M2／M3 的 Compose／页面验收仍需各自的
验证证据。
