# 控制会话

[English](control-session.md) | 中文

一个 Node 在唯一的监听端点上接受一个 Controller 控制会话。两种传输承载同样的帧体，运行同样的会话代码。消息本身见
[协议](protocol.zh.md)。相关决策是
[带 IPC 和 WebSocket 的帧通道](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/20260924-frame-channel-with-ipc-and-websocket.md)
和 [Controller 心跳与 WebSocket 关闭码](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/20260925-controller-heartbeat-and-websocket-close-codes.md)
两份 ADR。

```json
{
  "control": {
    "controller_id": "deployment-controller",
    "listen": { "kind": "ipc", "path": "/home/node/state/control.sock" },
    "heartbeat_ms": 1000,
    "frame_timeout_ms": 10000
  }
}
```

| `listen.kind` | 用途 |
|---|---|
| `ipc` | 本地部署：直接放在 Node home 中的私有 Unix socket；每个帧体前面有一个四字节大端长度 |
| `mutual_tls_websocket` | 沙盒：`bind`、`path` 和 `tls`（证书、私钥、CA 和可选的固定对端证书）；每个帧体一条二进制消息 |
| `websocket` | 明文，只用于回环的测试夹具；生产可执行文件拒绝它 |

网络监听还要求 `process.workload_uid`，这样工作负载从不和管理身份共用一个用户。可选的 `target`（租户、Workspace、
沙盒、运行时代次）是平台分配的运行时范围：有效的 Controller 证书也不能选择别的租户，范围之外的运行时绑定会被拒绝。
在 WebSocket 上，文本消息、其他路径和超过 16 MiB 帧上限的消息都会被拒绝。在集群中，Controller 通过沙盒路由器访问
Node，见[沙盒路由](../operations/sandbox-routing.zh.md)。`control` 要求同时配置 `clone` 段；没有 `control` 时，
可执行文件只恢复它已经受理的工作。

## 归属

部署绑定的是一个持久的 ControllerId，而不是第一个连上来的对端，重新配置成另一个 owner 会失败。新的 clone 在受理的
同时原子地归属到这个 Controller（schema v4）；更早的未归属记录既不会重放给会话，也不会被它确认。

IPC 路径的私有目录和 Node 数据库租约保护端点恢复：只有同一 owner 的、拒绝连接的私有 socket 可以被替换；文件、
符号链接和仍在监听的端点都会保留。

一个连接拥有握手和控制槽位。其他连接被拒绝，不会替换它：IPC 直接关闭 socket，WebSocket 则完成升级后以 `4409`
（`control session busy`）关闭，因为路由器会转发关闭码，却会把 HTTP 拒绝变成看起来像 Node 不可达的网关错误。
IPC 对端无法区分"忙"和 ControllerId 不匹配。

## 关闭

任何一方有意结束已建立的 WebSocket 会话时都发送关闭码，这样路由器转发的是原因，而不是报告连接丢失：

| 关闭码 | 含义 |
|---|---|
| `1001` | 进程正在停止 |
| `1002` | 违反协议 |
| `4403` | 对端不是配置的另一方，或缺少需要的能力 |
| `4408` | 对端在 I/O 截止时间内什么都没发，或停止了读取 |
| `1011` | 发送方自身的失败（持久化或准入） |

接收方回应关闭，所以双方都不用等关闭握手超时；Node 在准入路径之外完成这次握手，所以迟迟不走的 Controller 不会让
下一个连接变成"忙"。IPC 不带关闭码，只是结束流。不管关闭码是什么，关闭都只表示"连接不可用"。

## 准入与 worker

Hello 协商协议版本、Node 身份和 incarnation 以及能力。会话接受已协商能力的命令、状态查询和精确的确认；不支持或
冲突的消息会关闭会话，Cloud 的变更必须使用受控的信封。

- 准入使用有界队列和可撤销的会话守卫。只有持久受理在守卫下进行。断线或撤销会丢弃尚未受理的排队工作，但不能取消
  已受理的 clone。读、写和准入回复都使用 `frame_timeout_ms` 截止时间，回复的计时从它对应的帧到达时开始。
- 应答准入的 worker 拥有 Node 数据库，从不等待 Git。每个要运行 Git 的 clone 步骤都在单独的 clone 执行器上一次运行
  一步，worker 在交出之前记录每一步的输入
  （[决策](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20260925-clone-git-effects-run-outside-the-admission-worker.md)）。
  因此 Git 等待远端时，状态、确认和重放仍能得到应答。停止时，未完成的步骤最多可以继续
  `shutdown_grace_ms + cleanup_timeout_ms`；剩下的部分在下次启动时根据记录的尝试处理。
- 读取从不等待 worker：流结束、WebSocket 关闭或 ping，以及被截断的帧都会立即被发现，即使 Git 正在运行，所以在
  clone 中途重启的路由器发出的关闭会被遵守，槽位会被释放。应答按请求顺序发出；最多允许 15 个请求尚未应答。超过之后，
  状态查询会被丢弃（Controller 会再次轮询），其他任何消息都会关闭会话。
- 心跳与 Git 和插件下载无关。有界的重放分页包含所有执行族中未确认的事件；状态回复从不确认。

## 存活

Node 的每帧读取截止时间同时也是 Controller 的存活截止时间。每个 `query_interval_ms` 周期，Controller 恰好发送
一帧：对待处理派发的状态查询，没有待处理的派发时则发送带有 `controller_id` 的 Controller `heartbeat`。Node 在读取
循环中处理这个心跳，不进入 worker 队列，如果 id 不是它的 owner 就结束会话。因此空闲的会话保持打开，而消失的
Controller 或半开的连接会在 `frame_timeout_ms` 内释放槽位。让 `query_interval_ms` 不超过 Node `frame_timeout_ms`
的一半；两者位于不同的进程，无法一起检查。双方都不发送 WebSocket ping，但都会回应对端的 ping。Controller 和 Node
必须运行同一个版本。
