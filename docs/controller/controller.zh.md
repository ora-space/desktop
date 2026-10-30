# Controller

[English](controller.md) | 中文

`ora-controller`（`apps/ora-controller`）在权威方和执行工作的 Node 之间搬运工作。它不做任何业务决定，自己也从不执行 Git。它有两种部署形态，在配置中一次性选定：**SQLite**，单 Node 的本地协调者；**cloud**，一个无状态的客户端，主动拨出到 Ora Cloud 并驱动 Workspace 沙盒。两种形态下可执行文件都不打开任何监听。可执行文件只支持 Linux。Agent 会话中继见 [Agent 中继](agent-relay.zh.md)；Cloud 契约见 [Cloud 契约](cloud-contract.zh.md)。

## 协调存储

协调逻辑只通过 `CoordinationStore` 访问持久状态：每个方法是一个原子的异步业务操作（`take_over_node_event`、`record_queried_result`、`original_dispatch`、`pending_dispatches`、`result`，以及适配器自己工作用的 `serve`），从不暴露事务、连接或表。部署时选定一个适配器，两者互不作为后备。

- **`SqliteStore`** 在私有的 `home_directory` 中保存 `ora-controller.sqlite3`。它的操作在阻塞线程池上运行，所以 fsync 从不阻塞异步运行时。数据库的 application id 为 `0x4f524143`、schema 版本为 1，有精确的 schema 和完整性检查，并在同级的 `ora-controller.sqlite3.lock` 上持有操作系统租约（放在数据库旁边，这样在 macOS 和 Windows 上不会与 SQLite 自己的锁冲突）。不同的 ControllerId 或未知的已有文件会被拒绝；没有从 HOME 推导的位置，没有重置，也没有自动重新绑定。它对账、接管并报告数据库中已有的 clone 派发；它不受理新的工作。
- **`CloudStore`** 不打开数据库，也不获取文件租约：每个操作都是对 [Cloud 契约](cloud-contract.zh.md)的一次调用，由 Cloud 在 PostgreSQL 中提交。受理属于 Cloud 的公开 API。

没有本地 clone 入口：Workspace clone 需要只有 Cloud 能固定的 Git Server 快照输入，没有 Cloud 的部署无法产生有效的输入（[快照 clone 输入与执行 ref 授权](https://github.com/ora-space/specs/blob/main/decisions/cloud/controller-integration/20261016-snapshot-clone-input-and-execution-ref-grants.md)）。`result(execution_id)` 读取持久化的终止 `ExecutionOutcome`（`ready { path, commit }` 或 `failed { reason, retained_path }`，加上 Node incarnation）；没有结果不代表失败。`pending_dispatches(node)` 列出没有持久结果的执行，重新连上的会话会持续查询它们。查询完成和事件交付共享一个接管事务，只有收到的事件才会产生 Ack，并且是在它的回执提交之后。重复的内容是幂等的；冲突的内容被拒绝，不予确认。

可选 Agent 实时预览由连接所有的 `session/previews.rs` 中继：每个 execution 只有一个最新快照，最多 64 个工作者，单次
下游调用有界等待，退出连接时一并取消。64 个名额限制并发执行，不是连接累计执行次数；可靠终态成功接管后，立即移除该执行的快照并取消工作者，迟到预览不能重建已结束执行的中继，其他活跃执行不受影响。该旁路不写持久协调数据、不产生 Node Ack、失败不影响可靠 Thread 接管。存储适配器
只有实现实际下游发布能力后才返回 `supports_thread_previews=true`。`CloudStore` 声明支持该能力，`SqliteStore` 保持关闭；
Cloud 适配器映射定型正文、截断标记与历史位置，并提供显式预览发布方法。发布方法校验 Node 消息和已登记的会话，拒绝已结束执行，携带当前租约发送一次替换快照；不经过持久提交重试，不产生 Ack。仅在 Controller 与 Node 双方协商同意后中继预览；旧 Node 继续使用可靠定型通道。

## 运行可执行文件

```text
ora-controller --config /absolute/path/controller.json [--single-node]
```

`ControllerRuntime::open(RuntimeConfig)` 支持嵌入，`run(shutdown)` 拥有重连循环但不安装信号处理器。`Service::start(DeploymentConfig, NodeHosting)` 组装运行时和一个可选的托管 Node。无效的参数和配置会在获取任何租约之前失败。

```json
{
  "controller": {
    "home_directory": "/home/node/controller",
    "persistence": { "kind": "sqlite" },
    "controller_id": "deployment-controller",
    "protected_state_directories": ["/home/node/state", "/home/node/process"],
    "nodes": [
      { "node_id": "deployment-node", "endpoint": { "kind": "ipc", "path": "/home/node/state/control.sock" } }
    ],
    "session": { "io_timeout_ms": 10000, "query_interval_ms": 1000 },
    "reconnect_ms": 1000,
    "timezone": "Asia/Shanghai"
  },
  "single_node": {
    "node_executable": "/opt/ora/bin/ora-node",
    "node_config": "/home/node/config/node.json",
    "ready_timeout_ms": 30000,
    "stop_timeout_ms": 30000
  }
}
```

- 把每个 Node、进程宿主和 guardian 的状态根目录都写进 `protected_state_directories`；IPC socket 的父目录也受保护，与 Controller 状态的任何重叠都会被拒绝。把 Node 的 owner 配置为这个 ControllerId。
- 沙盒里的 Node 通过平台的 WebSocket 路由器访问：`{ "kind": "websocket", "url": "wss://…", "headers": { "ate-target-actor": "atespace/sandbox-id" } }`。header 原样发送，承载厂商的寻址和平台凭据；握手仍会校验 `node_id`。
- `--single-node` 启动配置的 Node：它必须使用 `ipc` 端点，且 `control.controller_id` 和 `control.listen` 一致；Node 在自己的进程组中运行，Controller 最多等待 `ready_timeout_ms` 让端点就绪。进程宿主和 guardian 是前提，它不负责启动。托管的 Node 自行退出时，Controller 以失败退出。
- 正常关闭依次停止 Node 会话、适配器自己的工作（cloud 形态下有界地释放租约）、托管的 Node（`SIGTERM`，等待 `stop_timeout_ms`，从不 `SIGKILL`），最后释放 SQLite 租约。Controller 停止从不取消已受理的 Node 执行。

每个失败或丢失的 Node 会话都按类别记录日志（不可达、未知沙盒、被拒、忙、协议、身份不匹配、Node 沉默、断开），并在 `reconnect_ms` 后重试，不会让任何执行失败或被重新创建。会话协议见[控制会话](../node/control-session.zh.md)。

## Cloud 形态

```json
"management_tls": {
  "certificate_file": "/run/ora-management/controller.pem",
  "private_key_file": "/run/ora-management/controller-key.pem",
  "ca_file": "/run/ora-management/ca.pem"
},
"persistence": {
  "kind": "cloud",
  "endpoint": "https://cloud:8443",
  "claim_interval_ms": 1000,
  "substrate": {
    "effects_url": "https://sandbox-server:18000",
    "router_url": "wss://sandbox-server:18000/ora-node/v1",
    "atespace": "ora",
    "request_timeout_ms": 30000
  }
}
```

可执行文件拒绝没有 `management_tls` 或 endpoint 不是 `https://` 的 cloud 配置，也拒绝 effects URL 不是 `https://` 或路由器不是 `wss://` 的 `substrate`。Cloud 通过客户端证书认证 Controller；每次调用还以 `x-ora-controller-id` metadata 携带 `controller_id`，Cloud 把它记为租约和提交的持有者。通道是惰性连接的，所以 Cloud 不可达只是调用时权威方不可用，而不是启动失败。HTTP/2 keepalive 每 30 秒发一次 PING，10 秒内没有回应就关闭连接，所以悄悄断掉的连接会暴露出来。

### 租约、认领与派发

适配器的 `serve` 任务获取 Cloud 的全局租约，每十秒续租一次。每次写入都携带租约 epoch 和稳定的提交身份；只有丢失回复时才重传，并且用同一个身份，所以 Cloud 回放记录的响应，而不会把一个效果应用两次。Cloud 的判定在适配器内部一次性映射为：冲突（从不原样重试）、不可用（什么都没提交）、未知（回复丢失，只能用同一身份重试）和资格过期（丢弃租约，重新获取）。不持有租约时什么都不认领、不派发、不确认，Controller 也从不退回到本地写入。

认领到的工作被校验为 Node 命令，并在 Node 会话交付之前用 `RecordDispatch` 登记，每批最多 16 个，所以很长的队列也不会耽误续租。每个工作条目都指明它运行所在的 Workspace 沙盒（Agent 会话和 Revision 交付）；Cloud 不再交出租户级 clone，没有目标的条目记一次日志后留在 Cloud 中排队。没有 `substrate` 配置段时，只有在本进程中与静态 Node 的会话完成握手、证明配置的 `node_id` 确实是端点背后的那个 Node 之后，才开始认领；派发会永久指定它的 Node，所以登记在错误 id 下的工作会永远等待。

持有租约期间，Controller 保持一条 `Watch` 流：

| 流状态 | 何时进入 | 认领时机 |
|---|---|---|
| 无 | 启动；流以错误结束；租约 epoch 被丢弃 | 每 `claim_interval_ms` 一次，每次都尝试重新打开流 |
| 活跃 | Cloud 发来了响应头 | 打开时、每个 `WorkAvailable`/`OperationAvailable`，以及每次续租之后 |
| 排空 | Cloud 发来 `Drain` 或正常结束了流 | 从不；保留租约，会话继续 |

信号只是加快认领；归属在 `RecordDispatch` 提交时决定，续租后的兜底认领会拾起信号丢失的工作。排空表示 Cloud 实例正在停止：Controller 持续重试，在新的流打开时离开排空状态；如果正在服务的 Cloud 以 `UNAVAILABLE` 之外的状态拒绝流，就退回到周期性认领。

### Workspace 沙盒

配置了 `substrate` 时，Controller 还会驱动 Cloud 的 Workspace 操作（ADR [Controller 驱动 Workspace 沙盒](https://github.com/ora-space/specs/blob/main/decisions/controller/node-management/0-controller-drives-workspace-sandboxes.md)）。它用 `ClaimOperation` 一次认领一个操作（Cloud 交回最早可运行的那个并决定步骤顺序），记录每一步的 effect 计划，执行它，然后以认领时的版本推进或推迟操作：

| 步骤 | Controller 做什么 |
|---|---|
| sandbox | 执行 ensure effect，并带着证据（`sandboxInstanceId`、`nodeId`）推进 |
| node | 通过 `router_url` 以 `ate-target-actor: <atespace>/<externalId>` 打开 Node 会话，并等待 Cloud 把它列为已连接 |
| clone | 把 Cloud 冻结的快照输入（`clone_input`）原样交给 `RecordDispatch` 和 Node，就绪后推进 |
| plugin | 派发冻结的插件安装或删除计划，见 [Node 插件](../node/plugins.zh.md) |
| quiesce | 关闭沙盒的派发闸门，只有没有待处理或未解决的工作时才报告空闲 |
| terminate | 先停止 Node 会话，再执行 terminate effect |
| cleanup | 执行数据删除 effect |

`effects_url` 是 Substrate 的 effect API（`GET`/`PUT /effects/{id}`）。Controller 在写入 effect 之前先读取，超时或服务端错误之后重新读取同一个 id，所以回复丢失的 effect 永远不会以新身份发送第二次。effect 失败以 `external_failure` 推迟，超时以 `substrate_timeout` 推迟，终止未得到确认以 `termination_unconfirmed` 阻塞。Controller 从不用 Project 或 Workspace 的字段拼 clone 输入：快照中没有 `clone_input` 的 clone 步骤，或 Node 协议拒绝的输入（见 [仓库 clone](../node/repository-clone.zh.md#输入)），会以 `external_failure` 阻塞而不派发。只有 commit 等于输入的 `base_commit` 时才接管就绪的 clone；否则既不接管也不确认。

普通 operation 每轮只给不可用的 Node 一次不可续期的两秒握手宽限，Node 就绪检查、quiesce 与 clone 断连共用这次宽限。没有目标立即延后；目标已就绪立即继续。Controller 只提交 `node_unavailable` 原因，不指定重试秒数；Cloud 持久计算连续不可用轮次的 5/10/20/40/60 秒退避，只在真实推进时清零。会话和全局租约继续运行。

已经登记的 clone 跨轮保留原 execution、Node 和冻结输入，每轮按单调时钟最多等五秒，然后提交 `clone_result_pending`，由 Cloud 安排一秒后再领取。重连和 pending 回复都不续期。Controller 先接管已知结果；从 Cloud 持久的 execution `created_at` 起满三十分钟仍无结果，才请求 `blocked/clone_result_timeout`，Cloud 提交前重新核对登记时间和并发到达的结果。原执行的已知成功或失败晚到时，Cloud 自动接管；失败仍遵守原 clone 失败预算。Node 报告的 Unknown 继续按原未决结果规则保持 `clone_result_unknown`。等待到期不授权重派、清理或终止；force-stop 仍由独立任务处理。

沙盒 Node 目标只来自 Cloud 的沙盒行；NodeId 与静态 Node 冲突的沙盒被拒绝，Controller 从不自己重建沙盒。握手之后它用 `RegisterNode` 登记 Node incarnation，每十秒报告一次已连接，会话丢失时报告一次断开（Controller 自己停止它时不报告），并在登记新 incarnation 之前结束旧的。第一次持有某个租约 epoch 时，它调用 `ListLiveSandboxes`，为列出的每个沙盒启动会话，并停止不在列表中的会话，所以重启后的 Controller 在认领操作之前就会重新连上。路由细节见[沙盒路由](../operations/sandbox-routing.zh.md)。
