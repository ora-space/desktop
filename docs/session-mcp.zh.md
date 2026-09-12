# Session MCP

[English](session-mcp.md) | 中文

Ora 将已配置的 MCP 插件作为会话运行时输入交付。它们不属于 Effect Resource，也不通过工作区文件交付。所有 ACP `session/new` 与 `session/load` 路径共用 Session Setup 快照，包括首次启动、发送时恢复、恢复失败后的会话重建、切换 Agent、工作流执行和运行时刷新。

## ACP 注入

普通聊天自动选择当前已安装、静态声明有效且配置完整的 MCP 插件。配置未完成的插件被跳过，不影响其余插件。

工作流 Agent 节点采用冻结的 `mcps` 绑定作为显式白名单，只交付 `enabled: true` 的插件。空列表或缺少 `mcps` 的旧图均表示不使用 MCP。已选插件未安装、声明无效或配置不完整时，整个设置过程失败；未选择的插件在配置读取和能力检查之前就被过滤。

服务名称使用规范插件 ID（`<namespace>/<identifier>`），并按 ID 排序。快照必须整体成功，运行时不会发送部分 `mcpServers` 列表。

Stdio 映射为 ACP `McpServer::Stdio`，并重新检查命令是否为当前插件版本目录内的普通文件。HTTP 映射为 `McpServer::Http`，要求 Agent 声明 HTTP MCP 能力。`{ "context": "workspace" }` 被替换为会话的绝对工作目录，字面量 `"."` 保持不变。环境变量和请求头使用 ACP 名称/值列表。

非空集合要求 Agent 支持 `session/load`，因为运行中变更 MCP 集合只能通过这一消息交付。不支持时，在发送任何设置消息之前失败；恢复失败后的 `session/new` 重建也遵循相同要求。Agent 的自有重放机制或 Ora 注入的历史文本不能代替 MCP 快照。

## 运行时刷新

运行中的会话仅在内存中保存 Desired 与 Active MCP 版本。安装、更新、卸载插件，以及保存、清除、恢复插件设置时，都会发送不包含凭据的唤醒通知。空闲会话立即通过 `session/load` 刷新；正在处理请求的会话在当前轮次结束后刷新。刷新期间阻止新请求进入。成功后推进 Active；刷新中出现更新的 Desired 时保留待处理状态。失败只阻塞当前会话，下次发送时重试，不继续使用过期配置。已停止的会话不在后台刷新。

工作流会话在首次创建、恢复、重建和刷新时均保留节点选择。Actor 重新创建后，通过现有节点运行与会话的关联，读取运行所引用的发布快照。关联的执行数据缺失或无效时直接报错，不回退为自动发现。修改草稿不会影响已有运行的选择。

选中插件的版本和设置仍是实时输入，继续使用现有安全刷新边界。白名单以外的插件变化不会改变当前会话的 Desired 版本。编辑器开关用于配置后续运行，不用于即时修改正在运行的会话。

MCP 刷新、Skill Effect 变更和 Agent 替换共用 Agent Session Barrier，确保新请求等待安全时机。它们不共用 Effect 状态：MCP 不会变成 Effect Resource、Desired 或就绪信号。

## 诊断日志与 Agent 一致性

每次发送 ACP `session/new` 或 `session/load` 前，Ora 都会写入一条名为 `sending ACP session configuration` 的 INFO 日志。日志包含 Ora 会话 ID、Agent、已有时的提供方会话 ID、ACP 方法、选择模式、服务数量，以及所选插件的 ID、包版本、配置修订号和传输类型。命令、参数、环境变量、HTTP URL、请求头和设置值不会写入日志。

Agent 适配器应把传入的 `mcpServers` 列表视为该会话的完整集合。Ora 保留共享 Agent 进程模型，不为每个会话创建独立 OpenCode 进程。OpenCode 截至 1.18.30 仍会在进程范围保留通过 ACP 注入的 MCP 注册，因此 Ora 虽然正确发送空列表，OpenCode 仍可能向当前会话暴露同一进程中较早会话注册的服务。该 provider 一致性缺口由 [OpenCode issue #32371](https://github.com/anomalyco/opencode/issues/32371) 跟踪。

## 安全与兼容

设置值只能存在于配置存储、短暂的内存快照和发送给可信 Agent 的 ACP 消息中。不得进入 Effect、SQLite、工作区文件、日志、错误、UI DTO、版本摘要或 Agent 进程环境变量。日志只能包含上述不含秘密的版本身份信息。工作流只保存插件 ID 和开关。错误仅包含插件 ID、设置 ID、传输类型和稳定错误码。

Ora 不会为 MCP 创建、修改或删除 `.mcp.json`、OpenCode JSON/JSONC、所有权旁文件、Git 排除文件或其他工作区路径。已有用户 MCP 文件保持原样。本实现没有从未发布的文件物化方案迁移的步骤。
