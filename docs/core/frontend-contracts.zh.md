# 前端契约

[English](frontend-contracts.md) | 中文

`@ora/contracts` 是前端看到 Desktop IPC API 的唯一途径。Rust 拥有 DTO 形状、操作目录和生成的客户端接线；TypeScript 拥有共享的客户端执行逻辑和 Tauri 传输层。新增或删除操作的步骤见[修改代码](../development/changing-the-code.zh.md)。

## 端点目录

`xtask` 在 `xtask/src/frontend/namespaces/` 下按命名空间划分的模块里，把每个面向前端的操作声明为一个 `FrontendEndpoint`，包含：

- `operation_name`：扁平的线上标识（`createTask`）
- `namespace` 和 `member_name`：操作在客户端上的位置（`client.task.create`）
- `request_type` 和 `response_type`：TypeScript DTO 名
- `response_mode`：显式声明的 `Unary` 或 `Stream`，从不根据名称推断

目录只是生成用的元数据，不包含传输路由、查询或序列化细节；前端类型从不来自领域实体或适配层自己的结构体。

当前的命名空间：`project`、`task`、`workspace`、`session`、`appEvents`、`agentRuntime`、`agent`、`agentImport`、`skill`、`skillImport`、`fileSystem`、`gitIdentity`、`workflow`、`workflowRun`、`scheduledTask`、`plugin`、`developerMode`、`runtimeLogLevel` 和 `proxy`。

## 生成

`task export-contracts` 先运行 `cargo xtask export-contracts`，再用 `ts-to-zod` 从 `dto/error.ts` 派生 `error.schema.ts`。输出写入 `packages/contracts/src`：

| 输出                  | 内容                                                                                                                      |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `dto/`                | 每个契约族一个 `.ts` 文件，由 `ts-rs` 生成。内嵌 ACP 载荷的字段引用 `@agentclientprotocol/sdk`，不再生成第二份 ACP schema |
| `dto/error.ts`        | request id 和公开错误的判别联合                                                                                           |
| `endpoints.ts`        | 端点清单                                                                                                                  |
| `dto.generated.ts`    | 从 ts-rs 输出派生的 barrel，没有第二份类型到文件的映射                                                                    |
| `client.generated.ts` | 由命名空间和响应模式派生的静态、带类型的客户端工厂                                                                        |

生成的文件带有所有权头部。`client.ts`、`client-runtime.ts`、`transport.ts` 和 `index.ts` 是手写的。导出先渲染到新的临时目录，拒绝冲突，替换有变化的自有输出并删除已退役的输出；非自有文件会保留，新生成的文件名也不能覆盖它们。DTO 模块文件名用 kebab-case（`#[ts(export_to = "workflow-run.ts")]`）。

`task check:contracts` 把两层都渲染到临时目录，比较内容和生成文件清单，不读 Git，也不改动工作区。CRLF 会规范化为 LF；其余任何差异、缺失文件、过期文件或未跟踪的生成文件都会导致失败。`task test:frontend` 会先跑这项检查；它从不修复输出。

同一次导出也会生成插件协议绑定，见[插件系统](../plugins/plugin-system.zh.md#协议与-sdk)。

## 带类型的客户端

`createContractsClient(transport)` 返回一个按命名空间组织的客户端，类型从清单派生：`ContractsClient` 把每个端点的 `namespace`/`memberName` 映射成嵌套对象类型，生成的工厂会和它对照检查。重复的操作或成员会让生成失败。

每次调用时，客户端把操作名和完整的请求 DTO 交给注入的传输层，从不拆分、改名或重新序列化字段。`appEvents` 命名空间提供 `client.appEvents.watch({})`：流以 `Ready` 开始，之后携带尽力而为的失效通知：`SessionTitleUpdated`、`PluginStatusChanged`、`AgentModelsInvalidated`、`McpHealthChanged` 和 `WorkflowRunInvalidated`。每条通知只说明什么变了，客户端重新查询权威状态。

## 传输层

`ContractTransport` 有 `send` 和 `stream` 两个方法，都接受可选的 `AbortSignal`。流是冷的、只能消费一次：对同一个 `AsyncIterable` 迭代两次会抛出 `stream_already_consumed`。

Desktop 传输层（`apps/desktop/web` 中的 `createTauriTransport`）把操作名映射到 Tauri 命令，并通过 Tauri Channel 驱动流。Desktop 自有的 `src-tauri/bindings/` 目录提供 handler 路径和显式的 Webview 授权；导出时把它和逻辑目录合并，生成 `tauri-bindings.generated.ts`、Rust 命令注册、带类型的流路由和权限列表。这些适配层元数据都不进入公开清单。缺失、重复、未知或模式不符的 binding 会让生成失败。

不属于 SDK 流的原生事件有自己的路由。例如 marketplace 自动刷新状态由插件契约中的 `MarketplaceAutoSyncEvent` 声明，由 `apps/desktop/src-tauri/bindings/marketplace_sync.rs` 路由，并通过生成的监听代码和 serde 样例纳入 `check:contracts`。

`watchAppEvents` 是失效广播，不是可回放的日志。Web App Shell 在挂载查询或打开这条流之前，平台适配层会先获取同源的 `ora:app-window` Web Lock，因此第二个浏览器标签页会等到第一个关闭。Desktop 适配层直接授予所有权，因为原生宿主只有一个主窗口。

## 错误与本地化

`decodeRemoteError` 先校验 `{ code, params, requestId }` 的形状和 UUID，再检查生成的联合类型：

- 已知 code 且参数有效：`RemoteContractError`。
- 未知 code 但 request id 有效：`UnknownRemoteError`，保留原始 code。
- request id 缺失或无效，或已知 code 的参数无效：`LocalTransportError`，kind 为 `malformed_response`。
- 网络、调用、分帧、流消费、不支持的操作和取消等失败使用有限的本地 kind，从不编造 request id。

App Shell 通过唯一的 `localizeContractError` 路径本地化所有错误，背后是中英文的 `errors.<code>`、`errors.unknown` 和 `errors.transport.<kind>` 资源。内部错误和未知错误会显示 request id 方便支持排查，本地传输错误则不显示。UI 组件从不渲染技术性的 `Error.message`，有一条 lint 规则强制这一点。`@ora/chat` 的 store 与语言无关，并以文本形式保存失败信息，因此宿主把 `describeChatError` 作为它的 `describeError` 选项传入：会话记录里的合约错误和传输错误走同一条本地化路径，而 agent 自己返回的失败文本保持原样。

## 包测试

contracts、chat 和 editor 包先运行 `tsc --noEmit`，再在 clean-stderr 包装下用 `deno test --no-check` 执行测试。相对导入写明 `.ts` 扩展名和目录入口文件，生成过程也保留这些扩展名，所以测试不需要宽松的导入解析，也不需要中间的 JavaScript 产物。
