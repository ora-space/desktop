# 本机 Controller 运行时

[English](local-runtime.md) | 中文

`ora-controller` 负责本机 clone 意图的持久接受与结果接管，其可执行入口同时承载
[minicloud](../minicloud/runtime.zh.md) 调用的过渡 clone API。它不执行 Git、不替换 Backend 写入入口，
也不充当 Cloud 权威存储。Linux 会话使用现有 [Node IPC](../node/local-ipc.zh.md)
和长度前缀 JSON 消息，不增加应用凭据。

## 接受与存储

协调逻辑只通过 `CoordinationStore` 接口读写持久状态：接口按完整原子业务操作定义
（`take_over_node_event`、`record_queried_result`、`original_dispatch`、`pending_dispatches`、`result`，
以及适配器与其权威方自有协调的 `serve`），异步形态，不暴露事务、连接或表。两个适配器实现它，在部署期
选定其一，不互为后备：本机部署用 `SqliteStore::open(home, controller_id)`，每个操作在 blocking pool 上
执行，SQLite 的 fsync 不占用承载 Node 会话与 API 的异步运行时；云端部署用 `CloudStore::open(&config)`，
每个操作是对 [Controller–Cloud 契约](../protocols/controller-cloud-contract.zh.md) 的一次调用，由 Cloud
在 PostgreSQL 中提交。接受调用方请求与目录列表（`accept_request`、`operations`、`operation`）是独立的
`CloneIntake` 接口，只有 SQLite 适配器实现：云端部署的接受入口属于 Cloud 公开 API，所以 JSON 表面根本
不会被组合。`result(execution_id)` 以 `ExecutionOutcome`（Node incarnation 加 `ready{path, commit}` 或
`failed{reason, retained_path}`）报告终态，这是两种权威存储共同持久的形状；本机目录保留完整线上结果供展示。

`accept_request(request_id, spec)` 返回的命令包含稳定 operation／execution。完整输入及目标 Node 落盘后
才返回；相同请求返回原命令，改变输入则拒绝。`result(execution_id)` 查询持久终态，没有结果不表示失败。
`pending_dispatches(node)` 只列出尚无持久结果的执行：它们是重连后会话周期查询的对象；已完成执行
重放的事件仍经 `original_dispatch` 校验。

显式注入的私有目录保存 `ora-controller.sqlite3`，与 Node／process 状态独立。
application ID 为 `0x4f524143`、schema version 为 1；精确结构／完整性校验和同级文件
`ora-controller.sqlite3.lock` 上的 OS 租约保护重开。租约放在数据库旁边，避免在 macOS 或 Windows 上
与 SQLite 自身的文件锁冲突。
不同 ControllerId 或未知已有文件会被拒绝。不从 HOME 推导目录，不清库，不导入历史任务或自动重绑定。

`clone_operations` 保存接受记录和不可变终态，`clone_receipts` 保存 Node 原事件精确身份与内容。
查询完成和事件交付使用同一接管事务；`take_over` 只对实际收到的事件、且在 `take_over_node_event`
返回后才构造 Ack，查询结果经 `record_queried_result` 保存但不产生 Ack 依据。
相同内容幂等，冲突输入／结果／请求关联不确认。历史结果保留原 Node incarnation，
查询报告者和心跳则必须匹配当前会话。

## 独立可执行入口

`ControllerRuntime::open(RuntimeConfig)` 支持内嵌。`handle()` 提供持久 clone 接受、操作列表和查询；
`run(shutdown)` 拥有重连循环，不安装进程信号。查询不存在与操作已接受但尚无终态明确区分。
库本身不依赖任何监听器；`Service::start(DeploymentConfig, Transport, NodeHosting)` 为可执行入口和测试
组合 API 监听、唯一运行时所有者以及可选托管的 Node。

构建 `cargo build -p ora-controller -p ora-node -p ora-process-host -p ora-process-guardian`。
部署状态放在一个配置文件里，本次进程的组合方式由命令行给出：

```text
ora-controller --config /absolute/path/controller.json [--single-node]
               [--transport tcp|unix] [--host 127.0.0.1] [--port 4820] [--socket /path/api.sock]
```

| 参数                      | 规则                                                                                                                                      |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `--transport tcp`（默认） | `--host` 默认 `127.0.0.1`，`--port` 默认 `4820`。非回环地址允许启动但会记录警告：API 没有认证，回环只是部署约束而不是安全保证。           |
| `--transport unix`        | 需要 `--socket`，必须是直接位于 `home_directory` 内的绝对路径，按与 Node endpoint 相同的私有 socket 规则创建；不接受 `--host`／`--port`。 |
| `--single-node`           | 按 `single_node` 段启动配置的 Node，正常关停时停止它，见下文。                                                                            |

非法参数组合与配置都在获取数据库租约前拒绝。

`persistence` 在部署期选定持久适配器；运行中不切换，两者互不为后备。`{ "kind": "sqlite" }` 用
`home_directory` 内的 SQLite 与文件租约并提供 JSON 表面，因此 `api` 段必填。云端形态不开库、不取文件租约、
不提供 JSON 表面，因此 `api` 必须缺省，监听器参数会被拒绝：

```json
"persistence": {
  "kind": "cloud",
  "endpoint": "http://127.0.0.1:8082",
  "claim_interval_ms": 1000
}
```

`endpoint` 是 Cloud 的 gRPC 地址；连接惰性建立，Cloud 不可达是调用时的"权威不可用"，不是启动失败。
当前阶段不认证 Controller：每次调用以 `x-ora-controller-id` metadata 携带 `controller_id`（须为可打印
ASCII），Cloud 把它记为租约与提交的持有者。`nodes` 必须恰好一个 Node：适配器的 `serve` 任务获取 Cloud 的全局租约、每十秒续期、
领取已接受的工作、按 Node 命令校验后用 `RecordDispatch` 登记派发，再由现有 Node 会话经周期状态查询交付。
一次领取连续登记直到 Cloud 没有工作或某一步失败，单批最多 16 项，长队列不会推迟租约续期。每个写操作携带租约 epoch 与稳定的提交身份；只有回复丢失时才重传，且用同一身份，
让 Cloud 回放已记录的响应而不是重复施加效果。Cloud 的裁决在适配器内一次映射为接口的错误分类：冲突
（不按原样重试）、不可用（未提交，稍后重试）、未知（回复丢失，只能同身份重传）、资格失效（忘记租约，
由下一次续期重新获取）。未持有租约时不领取、不派发、不确认，Controller 从不退回本机写入。

持有租约期间，Controller 维持一条 `Watch` 流，由流的状态决定何时领取：

| 流状态 | 进入条件                                  | 领取                                                                  |
| ------ | ----------------------------------------- | --------------------------------------------------------------------- |
| 无流   | 启动；流以错误状态结束；租约 epoch 被丢弃 | 每个节拍先尝试重开流，并每 `claim_interval_ms` 领取一次               |
| 在线   | 流建立（Cloud 已发送响应头）              | 建立时领取一次，每个 `WorkAvailable` 领取一次，每次续约后兜底领取一次 |
| 排空   | Cloud 发送 `Drain` 或正常结束流           | 不领取；保留租约，Node 会话照常                                       |

因此 `claim_interval_ms` 只是无流时的领取间隔；流在线时，空闲 Controller 每次续约查询 Cloud 一次，而不是
每个间隔一次。信号只加速领取：归属仍由 `RecordDispatch` 提交决定，续约后的兜底领取会补上 Cloud 因订阅者
缓冲满而丢弃信号的工作。排空表示该 Cloud 实例即将停止；Controller 在两个节拍上持续重开流，新流建立即结束
排空；若仍在服务的 Cloud 以 `UNAVAILABLE` 以外的状态拒绝流，则转入无流、按周期领取，而不是一直暂停。
Cloud 通道每 30 秒发送 HTTP/2 保活 PING（空闲时也发送），10 秒无回应即关闭连接，使被静默丢弃的连接让流以
错误结束，而不是看似在线。Cloud 从 `ora-space/cloud` `34d8067`（cloud#29）起才接受这一间隔；更早的 Cloud
会在空闲约两分钟后以 `GOAWAY` 断开连接。

```json
{
  "controller": {
    "home_directory": "/home/node/controller",
    "persistence": { "kind": "sqlite" },
    "controller_id": "deployment-controller",
    "protected_state_directories": ["/home/node/state", "/home/node/process"],
    "nodes": [
      {
        "node_id": "deployment-node",
        "endpoint": { "kind": "ipc", "path": "/home/node/state/control.sock" }
      }
    ],
    "session": { "io_timeout_ms": 10000, "query_interval_ms": 1000 },
    "reconnect_ms": 1000,
    "timezone": "Asia/Shanghai"
  },
  "api": { "node_id": "deployment-node" },
  "single_node": {
    "node_executable": "/opt/ora/bin/ora-node",
    "node_config": "/home/node/config/node.json",
    "ready_timeout_ms": 30000,
    "stop_timeout_ms": 30000
  }
}
```

`api.node_id` 指定已接受 clone 派发到的 Node，调用方不选择 Node。沙盒中的 Node 改为经平台
WebSocket 路由访问：

```json
{
  "node_id": "sandbox-node",
  "endpoint": {
    "kind": "websocket",
    "url": "wss://router.example/ora-node/v1",
    "headers": { "ate-target-actor": "atespace/sandbox-id" }
  }
}
```

请求头原样发送，厂商寻址和平台凭据只放在这里，握手仍校验 `node_id`。`ws://`／`wss://` URL 以及
请求头名称和取值在开库前检查。每次会话失败或断开都会按类别（不可达、沙盒不存在、被拒绝、会话占用、
协议错误、身份不符、Node 静默、连接断开）记录日志，并在 `reconnect_ms` 后重试，不判定执行失败，也不重建执行。“会话占用”只有 WebSocket 能识别（Node 以
close code `4409` 关闭）；IPC 的 Node 只能关闭 socket，会话占用会被记为协议错误。
经 WebSocket 时，Node 以 `1002` 或 `4403` 关闭分别记为协议错误和身份不符；Controller 自己结束会话时
也发送对应 code（停止为 `1001`，另有 `1002`、`4403`、`4408`，持久化失败为 `1011`），Node 和路由
都能看到会话结束的原因。

`protected_state_directories` 须列出所有 Node／host／guardian 状态根；配置的 IPC socket 父目录也受保护。Controller 数据目录与它们
重叠时，在开库前拒绝。独立程序恢复已接受记录，配置文件和 stdin 不是业务命令通道。
不托管 Node 时分别部署 host 和 Node，Node 配置的归属须匹配 ControllerId。

`--single-node` 要求 `nodes` 恰好包含 `api.node_id` 这一个使用 `ipc` endpoint 的 Node。开库前，程序只读
读取 `node_config`，其 `control.controller_id` 或 `control.listen`（`ipc` 类型且路径相同）不匹配、
或 endpoint 上已有进程接受连接时拒绝启动。随后在
自身进程组内（不新建会话）启动 `node_executable <node_config>`，在 `ready_timeout_ms` 内等待 endpoint
可连接，然后才绑定 API。process host 与 guardian 是前置条件，程序不部署也不启动它们。Controller
单独退出不会向 Node 发送任何信号，已接受的 clone 继续执行；运维或启动器按进程组停止时两者都会收到。
托管的 Node 自行退出时，Controller 关停并以失败退出，而不是继续受理无法派发的请求。

正常关停顺序固定为：API 受理（有限等待在途请求）→ Node 会话 → 适配器自有协调（云端形态下有限时间内
释放租约，放在会话之后，避免在即将释放的租约下写入）→ 托管 Node（`SIGTERM`，最多等待
`stop_timeout_ms`，不升级为 `SIGKILL`）→ SQLite 形态的数据库租约。本进程停止从不取消 Node 已接受的执行。

JSON 接口是 [minicloud](../minicloud/runtime.zh.md#http-接口) 文档描述的过渡 clone API，
DTO 位于 `ora-contracts::controller_api`，只在 SQLite 持久模式下存在。面向 Cloud 的契约由 Cloud 仓库的
proto 定义，Controller 作为客户端拨出（见 [Controller–Cloud 契约](../protocols/controller-cloud-contract.zh.md)），
不向 Cloud 暴露任何服务。

## 验证与保留范围

真实 SQLite 测试经 `CoordinationStore` 与 `CloneIntake` 接口覆盖接受、独占、事务失败、查询／事件乱序、
重复接管、冲突事实，以及已完成执行退出周期查询。Cloud 适配器的裁决映射、同身份重传、消息翻译与流状态迁移
有单元测试；`apps/ora-controller/tests/cloud.rs` 以内存假 Cloud 驱动运行时，假 Cloud 经 `ora-controller-proto`
测试专用的服务端桩（`test-server` feature）提供真实契约，覆盖信号触发领取、流建立前已接受的工作、断流回退与
重开、排空、排空后流被拒绝、打开时 epoch 陈旧、批量登记，以及关停时关闭流并释放租约。运行时与可执行程序测试覆盖云端形态不建本机状态、不提供 JSON 表面、拒绝 `api` 段或监听器参数、Cloud
不可达时保持运行。它对真实 Cloud 的行为（租约、领取、派发、接管、重启不重复 clone）经 [minicloud 云端
形态](../minicloud/runtime.zh.md#云端持久模式)端到端验证，尚未自动化；
framed 会话测试覆盖 Unknown 重传有界，以及错误 Node 身份或缺少 clone 能力时在派发前拒绝。
独立 Controller–Node–host／guardian 测试执行真实 HTTPS clone，
截住 Ack 后在持久接管之后强杀 Controller，再离线重启，检查原结果、精确 Ack、Node outbox 清空和唯一变更 Run。
Node 自身 IPC 测试另覆盖 Node 重启与事件重放。

另有独立子进程运行生产 `run_session` 与真实 SQLite 所有者，仅注入提交前暂停点。
父进程确认没有 Ack，在接管事务仍打开时发送 SIGKILL，再重开数据库验证回滚及原意图不变。
随后由正常 Controller 可执行程序在 HTTPS 拒绝访问时接管 Node 重放的结果。
暂停点是持久化测试依赖（`WritePoint::Commit`），不是部署选项或协议扩展。

这不代表 Client／UI、Cloud、多 Controller 或恶意对端保证完成；全部队列压力、崩溃边界和部署组合
仍在 approved ADR 核心用例中跟踪。既有 Backend 入口及 Worktree 协调保持不变。
