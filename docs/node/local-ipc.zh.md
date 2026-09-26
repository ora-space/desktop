# Node 控制会话

[English](local-ipc.md) | 中文

Linux `ora-node` 可在唯一一个监听入口上接收 Controller 控制会话：本机部署使用私有 Unix socket，
经平台路由访问的沙盒使用 WebSocket。两者承载相同的帧体（`[type][JSON]`），运行同一份会话代码。
在 [clone 部署配置](repository-clone.zh.md)旁增加 `control`：

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

沙盒中改为监听 WebSocket upgrade：

```json
"listen": { "kind": "websocket", "bind": "0.0.0.0:9001", "path": "/ora-node/v1" }
```

IPC 在每个帧体前加 4 字节大端长度；WebSocket 一条 binary message 恰好承载一个帧体，拒绝 text
message、其他路径和超过 16 MiB 帧上限的消息。Node 不对 WebSocket 对端做鉴权：监听地址只能经
平台路由到达，由平台鉴权 Controller；平台凭据不得写入 Node 配置或镜像。WebSocket ping/pong
只证明最近一跳存活，端到端存活仍以协议心跳和 `frame_timeout_ms` 为准。

IPC path 必须直接位于注入的 Node home 内。私有目录与 Node 数据库独占锁保护 endpoint 恢复：
只替换同用户、私有且连接被拒绝的旧 socket，保留普通文件、符号链接和活动 listener。
启用此配置要求已配置 clone；不配置 IPC 时仍作为只恢复历史执行的独立程序运行。

部署指定并持久绑定 ControllerId，不由第一个连接者认领；更换归属配置会失败。
schema v4 在接受事务中为新 clone 保存归属；旧的未认领执行原样保留，不向此会话重放或允许其确认。
这是可信本机归属检查，不是密码学认证，也不隔离同 UID 恶意代码。

无论经哪种传输到达，一个连接占有握手／控制槽，其他连接被拒绝且不顶替旧会话：IPC 直接关闭 socket；
WebSocket 先完成 upgrade 再以 close code `4409`（`control session busy`）关闭，因为路由会原样转发
close code，却会把 HTTP 拒绝变成与 Node 不可达无法区分的网关错误。IPC 没有 close code，被拒绝的对端只会在握手阶段看到
socket 关闭；Controller 无法区分“会话占用”和因 ControllerId 不符被关闭，两者都记为协议错误，
并在 `reconnect_ms` 后重连。两端都不主动发送 WebSocket 保活 ping，只回复对端的 ping；双向的
心跳保证连接上持续有流量。Hello 协商现有版本、Node 身份／运行实例及
clone 能力；会话接收 clone、状态查询和精确确认，冲突或不支持的消息会关闭连接。
心跳独立于阻塞 Git 执行；Node 主动按有界分页重放未确认 clone 事件，查询回复不确认事件。

受理使用有界队列和可撤销会话门禁。门禁只覆盖持久受理，随后才运行 Git；断连或会话撤销丢弃尚未受理
的排队工作，不取消已经受理的 clone。读帧、写帧及命令受理回复均使用有限的 `frame_timeout_ms` 期限；
受理回复的期限从对应帧到达时起算。执行线程繁忙时，即使心跳正常，查询／命令会话也可能超时关闭；
重连查询原执行，不创建新尝试。

读帧从不等待执行线程：前面的请求等待回复时，会话照常读取下一帧，因此即使 Git 占用执行线程，
对端结束流、WebSocket close／ping 和半帧也都能及时发现。clone 中途重启的路由发出的 close 会被立即
处理并释放控制槽，重连不再被 `4409` 拒绝。回复按请求顺序发出。未回复请求最多 15 个（为重放保留
一个执行队列位置）；超出时丢弃状态查询，因为 Controller 按定时器轮询、会再次查询；其他消息则关闭
会话，由 Controller 重连后通过查询对账。

Node 的每帧读期限同时是 Controller 的存活期限。Controller 在每个 `query_interval_ms` 节拍恰好发送
一帧：有待确认的派发时发送状态查询，没有时发送携带 `controller_id` 的 Controller `heartbeat`。Node
在会话读循环内处理该心跳，不进入 worker 队列，因此不会排在 Git 之后；`controller_id` 不是本 Node
的归属时结束会话。空闲会话因此保持连接，而 Controller 消失或连接半开时，控制槽会在
`frame_timeout_ms` 内释放。`query_interval_ms` 应远小于 Node 的 `frame_timeout_ms`（不超过其一半）；
两者位于不同进程的配置中，启动时无法互相校验。Controller 与 Node 必须使用同一版本：旧 Node 会拒绝
Controller 心跳。慢读可被断开，随后重连恢复投递。正常停止先关闭受理，再执行原受管进程清理。

真实 WebSocket 测试让生产 Controller 会话连接生产 Node，接管 clone 结果，观察第二个连接收到 `4409`
拒绝，并拒绝身份不符的 Node。另一个测试让 clone 结果在 WebSocket 断线和 Node 重启后保持未确认，
验证原样重放，再由生产 Controller 会话接管并确认，随后确认该事件不再重放。

真实独立入口测试覆盖归属／重复连接拒绝、HTTPS clone、Node 强杀重启、原结果重放和精确确认。
[Controller 验收](../controller/local-runtime.zh.md)另外覆盖独立进程持久接管及 Ack 丢失恢复。

补充真实 socket 测试会在 clone 已接受后暂停 HTTPS，观察心跳，令排队命令超时，
再验证该命令仍为 Unknown，而原 clone 继续完成。另有测试在执行线程被暂停的 Git 占用时，验证 IPC 对端结束流和 WebSocket
close 会立即释放控制槽、WebSocket ping 得到应答，以及超出未回复上限的轮询不会断开会话、Git 结束后得到回复。半帧测试验证超时与重新受理；
慢读测试持续发送状态查询但不读取回复，直至断连，再重连读取完全相同的未确认结果。
没有为测试增加生产协议消息。
