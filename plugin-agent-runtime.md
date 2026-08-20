# Agent 插件运行接口（PluginKind == Agent）

本文定义 `ora.kind == "agent"` 类插件的运行契约：宿主如何拉起插件、如何与插件通信、
插件必须实现哪些方法、ACP 流量如何透传，以及失败与生命周期语义。

## 0. 背景与现状

Ora 目前只能跑 5 个硬编码的 agent CLI：`AgentCli` 是 `crates/domain/src/session.rs` 里的
闭合枚举，`crates/backend/src/agent_runtime/connection.rs` 为每个枚举值起一个 supervisor，
直接 spawn CLI 并在 stdio 上跑 NDJSON ACP。

插件侧的零件已经各自就位，但彼此没有连起来：

- `crates/plugin-manager/` 已能发现 `package.json` 里 `ora.kind == "agent"`、
  `ora.contributes.agents[]` 的插件包（`PluginKind::Agent` 是目前唯一合法 kind）。
- `crates/plugin-runtime/` 已实现 Deno 子进程生命周期、二进制帧、`ora/register` 握手与
  请求相关性 —— 但只支持"宿主发请求 / 插件回响应"单向，插件主动发的任何 method 都会被
  判为协议错误并杀掉进程。
- `crates/acp/` 已实现 ACP 相关性、会话响应定序、权限请求路由、trace —— 但传输写死成
  `AsyncRead` / `AsyncWrite` 上的 NDJSON。
- Desktop 已经 bundle 了 deno、已经在 bootstrap 调 `PluginManager::discover`，
  但结果只喂给了设置页的列表命令。

目标：让 agent 类插件成为一等的 agent 提供方 —— 宿主把 ACP 请求交给插件，插件转发给它自己
拉起的 agent CLI，响应原路返回；插件额外必须注册 start / stop / listModels 三个控制方法；
ACP 部分宿主不做任何校验，纯管道。最终插件替换内置 CLI。

## 1. 契约总览

一个 agent 插件与宿主之间只有两类流量，全部跑在 `plugin-runtime` 已有的二进制帧
（`[len i32 BE][type i8][json]`）之上：

```text
┌──────────────────────── Ora Host (Rust) ─────────────────────────┐
│  agent_runtime                                                   │
│    ConnectionSupervisor(AgentRef)                                │
│      │            ┌───────────────┐                              │
│      │  ACP 帧    │  AcpClient    │ ← 会话定序 / 权限路由 / trace  │
│      │            └───────┬───────┘                              │
│      │                    │ AcpTransport::send(Value)            │
│  ┌───┴────────────────────┴──────────────────────────────────┐   │
│  │              PluginRuntime（一个插件包一个）               │   │
│  └───┬───────────────────────────────────────────────────────┘   │
└──────┼───────────────────────────────────────────────────────────┘
       │  ① 控制：invoke   agent/start · agent/stop · agent/listModels
       │  ② 数据：notify   agent/acp（双向，payload 对宿主不透明）
       v
┌──────────────── Deno 插件进程（@ora-space/sdk）──────────────────┐
│   defineAgent({ start, stop, listModels, onAcp })                │
│      │  插件自己 spawn，自己管进程生命周期（见 §8）                │
│      v                                                           │
│   agent CLI（原生 ACP，或插件自行翻译的私有协议）                 │
└──────────────────────────────────────────────────────────────────┘
```

两条通道的定位差别，是后面所有取舍的根：

| 通道     | 形态                      | 相关性由谁负责                | 超时                        |
| -------- | ------------------------- | ----------------------------- | --------------------------- |
| 控制方法 | JSON-RPC request/response | `PluginRuntime` 的 `id`       | `call_timeout`              |
| ACP 透传 | JSON-RPC notification     | ACP 自己的 `id`（宿主不介入） | 无，由 ACP 层与会话取消负责 |

插件→宿主方向目前只有 notification 一种形态。反向 request/response（插件调用宿主能力）在
agent 契约里没有出现——ACP 的权限请求本来就走 ACP 自己的通道，其余宿主能力（storage、UI）
属于别的 plugin kind，等真有 agent 侧需求时再加，不预留空接口。

## 2. 进程与身份模型

**一个 agent 插件 = 一个 agent = 一个 Deno 进程。** codex 插件就是 codex，claude 插件就是
claude；想提供两个 agent 就发两个插件包。三者一一对应，是这份契约里最重要的简化：

- 帧里**不需要**任何寻址字段。`agent/acp` 的 `params` 直接就是原样 ACP 帧，宿主与插件之间
  不存在"这帧发给谁"的问题：

  ```jsonc
  // 宿主 → 插件（反向同形）
  { "jsonrpc": "2.0", "method": "agent/acp", "params": {/* 原样 ACP 帧 */} }
  ```

- 控制方法**不需要**参数里带 agent 身份（见 §4）。
- 崩溃半径天然等于一个 agent，不存在"一个进程死掉打掉同包其它 agent"的连坐（见 §9）。
- 宿主侧的 agent 身份**就是 plugin id**，不需要二元组：

  ```rust
  /// Identifies one installed agent provider; equal to the plugin package id.
  #[derive(Debug, Clone, PartialEq, Eq, Hash)]
  pub struct AgentRef(String);
  ```

同一个 agent 的多个会话仍然共享这一个进程，由 ACP 自己的 `sessionId` 区分——这是 ACP 早已
解决的问题，没必要在插件协议层再造一遍。

### manifest 随之收窄

`ora.contributes.agents[]` 的数组形态与"一个插件一个 agent"冲突，改为单对象：

```jsonc
"ora": {
  "kind": "agent",
  "id": "ora-space.claude",
  "displayName": "Claude Code",
  "contributes": { "agent": { "displayName": "Claude Code", "contractVersion": 1 } }
}
```

`InstalledPluginAgent` 随之去掉 `id` 字段（它恒等于 `ora.id`），`InstalledPlugin.agents:
Vec<_>` 变成 `agent: Option<InstalledPluginAgent>`——`kind == "agent"` 时必须存在，缺失即
校验失败。按 AGENTS.md 的"不保留兼容层"，数组形态直接删掉，不做二选一解析。

### 对现有代码的影响

- `crates/domain/src/session.rs` 的 `AgentCli` 闭合枚举被 `AgentRef` 取代，
  `from_database_value` 从"拒绝未知值"变成"接受任意合法 plugin id"——未知 agent 不再是数据
  损坏，而是"插件当前未安装"，属于正常的运行期状态。
- `ConnectionSupervisors` 的 5 个具名字段变成 `HashMap<AgentRef, ConnectionSupervisor>`，
  由 bootstrap 时的 `PluginManager` 快照构建。
- **DB 值不需要迁移**：`AgentCli::database_value()` 今天存的就是 `"ora-space.claude"` 这类
  命名空间字符串，只要 bundled 插件的 `ora.id` 沿用同名，历史会话的 `agent_cli` 列原样就是
  合法的 `AgentRef`。这让 §10 阶段 4 从"数据迁移"降级为"放宽校验"。
- 内置的 5 个 CLI 按 §10 的阶段迁移为随 Desktop 一起 bundle 的插件包，迁移完成后
  `AgentCli`、`cli_path.rs`、`launch_arguments()` 整体删除，不保留兼容分支。

## 3. 握手：`ora/register` 的扩展

现有握手只声明"我能被调用哪些方法"，无法表达"我会主动发哪些方法"，而 `handle_message`
把插件主动发的一切判为协议错误。扩展 registration 的 params：

```jsonc
{
  "jsonrpc": "2.0",
  "method": "ora/register",
  "params": {
    "methods": ["agent/start", "agent/stop", "agent/listModels"],
    "emits": ["agent/acp"],
  },
}
```

- `methods`：宿主可调用的方法，语义不变（一次性、不可变、重复注册即协议错误）。
- `emits`：插件可主动发出的方法白名单。**未在 `emits` 中出现的插件→宿主方法仍然是协议错误
  并杀进程**——放行任意主动消息会让协议退化成"什么都可能来"，而白名单让宿主在握手阶段就能
  拒绝一个行为超出声明的插件。

对 `kind == "agent"` 的包，宿主在握手完成时**立即校验**三个控制方法与 `agent/acp` 全部存在，
缺任何一个就直接判定插件不可用（不重启、标记 `Failing`）。这是 fail-fast 的关键：在没有会话、
没有用户等待的时刻暴露契约不符，而不是等用户点了发送才报错。

## 4. 控制方法

三个方法都是 request/response，走 `PluginRuntime::invoke`，受 `call_timeout` 约束。

### `agent/start`

```jsonc
// params
{ "cwd": "/Users/x",            // 宿主的 home_directory，等价现有 ProcessSpec::cwd
  "hostVersion": "0.8.0" }

// result
{ "protocol": "acp",            // 目前唯一取值，预留给非 ACP agent 的翻译层
  "acpVersion": 1 }
```

宿主在**插件进程 ready 之后、发出第一帧 ACP 之前**调用一次。返回成功即表示
"这个 agent 已准备好收 ACP 帧"；宿主随后立刻透传 `initialize` 走完 ACP 握手，拿到
capabilities 才把连接置为 `Ready`。

失败语义分两类，对应现有 `spawn_initialized_process` 的两种失败：

- `code = -32001 AGENT_NOT_INSTALLED`：底层 CLI 不在这台机器上。这是**预期的本地配置**，
  连接进入 `Unavailable` 并按退避重试，但**不打日志**（沿用 `AgentCliNotFound` 今天的处理，
  否则重试会刷屏）。
- 其它错误码：真实启动失败，记 `warn` 并退避重试。

### `agent/stop`

```jsonc
// params: {}   result: {}
```

用户禁用插件或宿主退出时调用。插件负责终止它持有的 CLI。宿主对 `agent/stop` 只等待
`shutdown_timeout`，超时后走进程级 `kill`——插件的清理是尽力而为，**进程树的最终回收责任
始终在宿主**（`ora-process` 已有该能力）。

`agent/stop` 与 `ora/shutdown` 是两件事：前者停 CLI、保留插件进程（便于随后 `listModels`
或重新 `start`），后者才结束插件进程。关闭流程是 `agent/stop` → `ora/shutdown`。

### `agent/listModels`

```jsonc
// params: {}
// result
{
  "models": [
    { "id": "claude-opus-5", "displayName": "Opus 5", "default": true },
    { "id": "claude-sonnet-5", "displayName": "Sonnet 5" },
  ],
}
```

为什么不复用 ACP 的 session config options：设置页要在**没有任何会话**的情况下展示可选模型，
而 config options 是 `session/new` 之后才有的会话级能力。两者共存——`listModels` 服务
"选 agent / 选模型"的前置 UI，config options 服务会话内切换。

## 5. ACP 透传

**宿主对 `params` 不做任何解析、校验、改写。** 因为一个插件只代表一个 agent（§2），连信封都
不需要——`params` 原封不动就是一帧 ACP：

```rust
// 宿主侧只有这一步：把 params 整体交给 AcpPeer / 把 AcpClient 要发的 Value 整体塞进 params。
let message: Value = notification.params;
```

- `params` 不是 JSON 对象 → 丢弃该帧并 `warn`，不杀进程。单帧格式错误不该让整个连接连坐。
- 帧内部无论是请求、响应、通知，还是宿主根本不认识的 ACP 扩展方法，一律原样投递。
  这是"纯管道"的字面含义，也是插件能先于 Ora 支持新 ACP 方法的前提。
- `agent/start` 之前到达的 ACP 帧同样丢弃并 `warn`：此时宿主还没有建立对应的
  `RuntimeConnection`，没有地方投递。

**为什么 ACP 用 notification 而不是 `invoke`**，三条独立理由：

1. **双层相关性是纯粹的负担。** ACP 帧自带 `id`，`AcpClient` 已经实现了会话定序、
   `PendingSessionRequest`、取消后的墓碑。再套一层 `PluginRuntime` 的 `id`，等于两套超时、
   两套取消、两处可能对不上的状态。
2. **`call_timeout` 会掐断长跑请求。** `session/prompt` 可以跑几分钟，而 `call_timeout` 是
   为控制方法设计的秒级超时。
3. **`session/update` 无处安放。** 流式更新是无请求的通知，request/response 模型里没有它的
   位置。

## 6. 传输抽象：`ora-acp` 从字节流解耦

`AcpPeer::spawn(reader, writer)` 与 `AcpClient<Writer: AsyncWrite>` 把 NDJSON 焊死在了传输层。
插件路径上 ACP 帧是**已经解析好的 `Value`**，再序列化成 NDJSON 字节、走一遍 `LinesCodec`、
再解析回来，是纯浪费。

引入消息级传输 trait：

```rust
/// Carries whole ACP JSON-RPC messages for one connection.
///
/// Implementations own framing and ordering: `send` must deliver complete messages in call order,
/// and the receiver handed to `AcpPeer::spawn` must yield exactly one message per frame. The peer
/// never inspects transport-level framing, so a transport may be a byte stream (NDJSON over stdio)
/// or an already-parsed channel (plugin IPC).
pub trait AcpTransport: Send + Sync + 'static {
    fn send(&self, message: Value) -> impl Future<Output = Result<(), AcpError>> + Send;
}
```

两个实现：

- `NdjsonTransport<W: AsyncWrite>`：现有行为原样搬过来，服务尚未插件化的内置 CLI。
- `PluginAcpTransport`：`send` 即一次 `PluginRuntime::notify("agent/acp", envelope)`。

`AcpClient<Writer>` 的泛型参数由 `Writer` 改为 `Transport`。backend 侧需要一个具体类型
（`RuntimeConnection` 存在 `watch` 里，不能是泛型），用 enum 而非 `Box<dyn>`，保持静态分发
与穷尽匹配：

```rust
/// Selects the transport that carries one connection's ACP traffic.
enum AgentTransport {
    Stdio(NdjsonTransport<ChildStdin>),
    Plugin(PluginAcpTransport),
}
```

`RuntimeConnection.client` 变为 `AcpClient<AgentTransport>`，backend 其余部分不感知差异。

这个 enum 是过渡期产物：§10 阶段 5 把内置 CLI 全部插件化之后 `Stdio` 分支再无使用者，
连同 `NdjsonTransport` 一起从 backend 删掉，`RuntimeConnection` 直接持有
`AcpClient<PluginAcpTransport>`。

## 7. `plugin-runtime` 需要的改动

现有实现是严格单向的（宿主发请求、插件回响应），改动四处：

1. **inbound 通道。** `launch` 的返回从 `PluginRuntime` 变为
   `(PluginRuntime, mpsc::UnboundedReceiver<PluginNotification>)`，形状对齐 `AcpPeer::spawn`。
   用 unbounded 的理由与 `ora-acp` 相同：连接级背压会让一个吵闹的会话拖垮同进程的其它会话，
   真正的有界队列在 `RouteRegistry` 的 per-session 层。
2. **`handle_message` 放行白名单内的通知。** 当前"含 `method` 即协议错误"的分支改为：在
   `emits` 内 → 投递到 inbound；不在 → 维持协议错误。
3. **`notify(method, params)`。** 宿主→插件的无 id 消息，复用同一个 writer 任务，不占用
   `pending` 表。
   `call_timeout` 的作用域收窄为"只管 `invoke`"，`notify` 不受它约束。
4. **控制请求超时墓碑。** `invoke` 超时后从活动 `pending` 表移除请求，但在 256 项有界队列中
   保留请求 id。插件迟到的合法响应会被识别为“已知但已放弃”并静默丢弃；真正未由宿主发出的
   id 仍是协议错误。这样既不会因一次本地超时误杀健康插件，也不会让历史 id 无限占用内存。

**不做**反向 request/response（插件发带 `id` 的请求给宿主）。agent 契约里没有任何一处需要它，
按 AGENTS.md 的原则不预留将来"可能会用"的通路；真需要时再加，届时插件发来的带 `id` 消息进
inbound、由上层 `respond(id, result)` 回写即可，与现有 `pending` 表互不干扰。

## 8. 子进程：插件自己拉起

**插件自己 spawn 它的 agent CLI，自己负责这个子进程的整个生命周期。** 宿主不代管、不代拉、
不在数据路径上碰 CLI 的 stdio——它只看见 `agent/acp` 帧。

对应到启动参数：agent kind 的插件进程需要 `--allow-run`（以及 CLI 所需的 `--allow-read`、
`--allow-env`、`--allow-net`）。也就是说**当前阶段不对 agent 插件做能力收敛**，一个 agent
插件拿到的权限约等于宿主自身。

这与 `plugin-sandbox-notes.md` 的"插件不获得 `--allow-run`、由宿主代拉起并套 OS 沙箱"结论
不一致。这里是有意的取舍：沙箱本阶段不做，先把 agent 契约跑通。留一条记录，便于以后恢复讨论
时知道要动哪几处：

- `agent/start` 的返回值加一个 `transport` 描述，让插件可以选择"把启动描述交给宿主、自己退出
  数据路径"（此时宿主用 `NdjsonTransport` 直连 CLI，插件只服务 `listModels`/`stop`）；
- 或者保留插件在数据路径上，但把 spawn 换成宿主代理能力（`process/spawn` 等），届时才需要
  §7 提到的反向 request/response。

两种收紧方式都只影响 `agent/start` 的返回值与 transport 的选取，**不改变 `agent/acp` 的管道
语义**，所以现在不实现也不会把设计钉死。

一个直接后果：插件永远在数据路径上，`PluginAcpTransport` 是插件 agent 的唯一传输形态。

## 9. 生命周期与失败语义

单个 agent 的状态机在现有连接状态上增加 `Failing`，用来区分“暂时不可用、仍会重试”和
“短时间连续失败、已停止重试”：

```text
                 插件进程 ready (ora/register)
                          │
                          v
   ┌──────────┐  agent/start ok   ┌──────────┐  ACP initialize ok  ┌────────┐
   │ Starting │ ────────────────► │ Starting │ ──────────────────► │ Ready  │
   └────┬─────┘                   └────┬─────┘                     └───┬────┘
        │ AGENT_NOT_INSTALLED          │ 其它错误                      │ 进程/协议失败
        v                              v                              v
   ┌─────────────┐               ┌─────────────┐                ┌─────────────┐
   │ Unavailable │◄──── 退避重试 ─│ Unavailable │◄───────────────│ Unavailable │
   └─────────────┘               └─────────────┘                └─────────────┘

   任意真实启动/连接失败 ── 1 分钟内超过 3 次 ──► Failing（停止自动重启）
```

**崩溃半径就是一个 agent。** 一插件一 agent 一进程（§2）意味着这里没有扇出、没有连坐，
插件进程死掉时的处理与今天 CLI 进程死掉时完全同构：

1. `active_generation` 归零，`state → Unavailable`；
2. `routes.fail_generation(generation, error)` 唤醒所有等待中的会话请求；
3. `mark_running_sessions_stopped` 把 DB 里 running 的会话落到 stopped。

这三步现有 `run_supervisor` 已经做了，插件路径只是把"CLI 进程退出"换成"插件进程退出"作为
触发源，控制流一行不改。

**熔断**沿用 `plugin-runtime.md` 已定的策略：连接 supervisor 跨进程代际记录真实启动失败和
连接崩溃，1 分钟内超过 3 次就把连接标记为 `Failing`，停止自动重启并在 UI 提示。计数必须放在
连接 supervisor，而不是单代的 `PluginRuntime` 内：每次重启都会创建新的 runtime，只有前者能
看见跨代际历史。`AGENT_NOT_INSTALLED` 是可恢复的本地配置，不计入熔断；契约缺失属于确定性错误，
无需等待阈值，直接进入 `Failing`。

**关闭顺序**：`agent/stop` → `ora/shutdown` → 等 `shutdown_timeout` → `kill` 进程树。任何一步超时都直接进下一步，不阻塞宿主退出。

这个顺序也适用于尚未进入 `Ready` 的失败路径：stdio 不完整、注册超时、契约校验、
`agent/start`、`agent/listModels` 或 ACP 初始化失败，都必须等当前插件进程树完全退出后才能安排
下一代。插件意外退出时，`PluginRuntime` 会主动关闭上行通知流；即使 ACP transport 仍持有运行时
句柄，连接 supervisor 也能立即观察到代际结束并执行会话失败与熔断逻辑。

## 10. 落地阶段

每一阶段都能独立提交、独立回归：

1. **`plugin-runtime` 双向化**（§7）。纯 crate 内改动，用 duplex 管道做单元测试，不碰 backend。
2. **`ora-acp` 传输抽象**（§6）。引入 `AcpTransport` + `NdjsonTransport`，现有 5 个 CLI 切到
   新 API。这一步应该是零行为变更的重构，由现有 ACP 测试兜底。
3. **manifest 收窄**（§2）：`contributes.agents[]` → `contributes.agent`，`plugin-manager`
   的校验与 `InstalledPlugin` 随之调整。改动小、无运行期依赖，越早做越省返工。
4. **agent 插件运行时**：`agent/start|stop|listModels` + `agent/acp` + `PluginAcpTransport` +
   `AgentTransport` enum，跑通"插件提供的 agent 出现在 UI 并可对话"。此时内置 CLI 仍走老路径，
   二者并存。
5. **身份泛化**（§2）：`AgentCli` → `AgentRef`，`ConnectionSupervisors` 改 map。DB 值不变，
   只是把"闭合枚举校验"放宽为"任意 plugin id"，不需要数据迁移。
6. **内置 CLI 插件化**：5 个 CLI 各出一个 bundled 插件包（`ora.id` 沿用现有 database value，
   自己 spawn 对应的 CLI），删除 `cli_path.rs`、`launch_arguments()`、`AgentCli` 残余，以及
   `AgentTransport::Stdio` 与 `NdjsonTransport` 在 backend 的使用。

阶段 4 与 6 之间是唯一的"两套路径并存"窗口，应尽量短。

## 11. 测试策略

- **协议层**（`plugin-runtime`）：`duplex` 管道 + 手写帧，覆盖 `emits` 白名单、未声明方法杀
  进程、`notify` 不占用 pending 表、插件通知与响应交错时的相关性、控制请求超时后的迟响应
  被忽略但真正未知 id 仍失败，以及关闭等待完整进程回收。
- **传输层**（`ora-acp`）：同一组 ACP 场景分别跑 `NdjsonTransport` 与内存 transport，断言两者
  产生**完全相同**的 `AcpInboundEvent` 序列（整对象 `assert_eq!`）。
- **契约层**：一个 fixture 插件（几十行 TS）实现三个控制方法与一个 echo agent，在集成测试里跑
  完整 start → initialize → prompt → stop，断言会话事件序列。
- **失败注入**：fixture 插件在指定时机退出／回错误码／超时，断言连接状态迁移与 DB 里会话落到
  stopped；纯状态机测试覆盖“1 分钟内第 4 次失败熔断”和窗口外旧失败过期。

`tracing` 断言按 AGENTS.md 的要求统一走 `with_trace_logging`。

## 12. 待定问题

1. **`agent/start` 的参数面。** 当前只给 `cwd`，但真实 CLI 常需要按工作区切目录、按项目注入
   环境变量。是每个 Ora 会话一次 `start`（重，但隔离干净），还是一次 `start` 服务所有会话、
   由 ACP 的 `session/new` 携带 cwd（轻，但插件要自己做多路复用）？倾向后者——ACP 本来就有
   会话级 cwd。
2. **模型列表的缓存与刷新。** `listModels` 要不要缓存？插件侧模型变化（登录态切换）如何通知
   宿主？可能需要一个 `agent/modelsChanged` 通知加进 `emits`。
3. **`AgentRef` 未安装时的 UI 语义。** 历史会话引用了一个已卸载插件的 agent，应该只读展示、
   提示安装、还是允许改派到别的 agent？
4. **bundled 插件的信任等级。** 随 Desktop 分发的内置插件是否跳过签名校验？如果跳过，"内置"
   这个属性由谁断言（安装路径？签名？manifest 字段？）。沙箱不做的前提下，这条决定了第三方
   agent 插件与内置插件在权限上有没有区别——目前的答案是没有区别。
5. **CLI 崩溃与插件进程崩溃如何区分。** 插件自己管 CLI 子进程，宿主只能看到"ACP 不再有响应"。
   插件应该在 CLI 意外退出时主动发什么（一个 `agent/exited` 通知？还是靠 ACP 层超时）？这决定
   了会话能不能给出"agent 崩溃了"而不是"卡住了"的提示。
