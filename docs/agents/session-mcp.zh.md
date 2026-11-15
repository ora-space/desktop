# Session MCP

[English](session-mcp.md) | 中文

Ora 把已配置的 MCP 插件作为**会话运行时输入**交付：也就是每次 `session/new` 和 `session/load` 中 ACP 的 `mcpServers` 列表。MCP 从不是 Effect Resource，也从不是 Workspace 文件。每个建立 provider session 的帧（`startSession`、prompt 触发的附着、重建、切换 Agent、工作流启动和实时刷新）都使用同一份会话 setup 快照。纯解析逻辑在 `crates/agent-runtime/src/mcp_resolver`，Desktop 数据来源和健康探测仍在 Backend。

## 选择

每个会话持久化一个 `SessionMcpSelection`：

- **Automatic**（普通聊天）：所有已安装、静态有效且配置完整的 MCP 插件。配置不完整的插件被排除，不影响其他插件。
- **Explicit**（工作流 Agent 节点）：只有节点 `mcps` 绑定中 `enabled: true` 的那些。空列表不交付任何 server。选中的插件缺失、无效或配置不完整时 setup 失败；未选中的插件在任何检查之前就被过滤掉。没有 `mcps` 的图表示空选择。

安装 MCP 插件只是把它加入全局目录；工作流节点的选择才是授权决定。选择在会话创建时存入会话行，并在恢复、重建、切换 Agent 和刷新时复用；恢复从不根据工作流元数据重新构造它，所以元数据缺失不会扩大显式选择。迁移 `0011` 之前就存在的会话得到空的显式选择。编辑工作流草稿从不改变已有的运行。

## 构建快照

server 名称是规范插件 id（`<namespace>/<identifier>`），按 id 排序。快照要么完整要么没有：运行时从不发送部分列表。

- stdio 映射为 ACP `McpServer::Stdio`；命令会被重新检查，必须是当前包版本里的普通文件。
- HTTP 映射为 `McpServer::Http`，要求 Agent 声明 HTTP MCP 能力（否则 `mcp_http_capability_missing`）。
- `{ "context": "workspace" }` 变成会话的绝对 cwd；字面量 `"."` 保持 `"."`。环境变量和 header 变成 ACP 的 name/value 列表。
- 非空集合要求 Agent 支持 `session/load`，因为这是唯一能把变化后的集合送达运行中会话的帧。如果 Agent 不能 load 会话，setup 在发出任何帧之前就失败，包括重建时的 `session/new`：重建要么带着真实快照，要么不发生。

## 实时刷新

活跃会话在内存中保存 Desired 和 Active 两个 MCP revision。插件的安装、更新和卸载，以及插件设置的保存、清除或恢复，都会发送一个不含机密的唤醒。空闲会话立即 `session/load`；忙碌的会话在当前 prompt 之后刷新，新 prompt 等待刷新完成。成功会推进 Active revision，期间到达的更新的 Desired 保持待定。失败只阻塞这一个会话，下一个 prompt 会重试，而不是使用旧配置。已停止的会话不在后台刷新。显式选择之外的插件变化不会改变该会话的 revision。

MCP 刷新、Skill Effect 变更和 Agent 替换共用一个 Agent Session Barrier，让新 prompt 等到安全点。它们不共享任何 Effect 状态。

## 机密与文件

设置值可以存在于配置存储、短暂的内存快照和发给 Agent 的 ACP 帧中。它们从不进入 Effect 状态、SQLite、Workspace 文件、日志、错误、UI DTO、revision 摘要或 Agent 环境变量。错误只给出插件 id、设置 id、传输方式和稳定的 code。

Ora 从不为 MCP 创建、修改或删除 `.mcp.json`、OpenCode 的 JSON/JSONC、附属文件、Git exclude 文件或任何其他 Workspace 路径；用户自己写的 MCP 配置保持原样。

## 诊断

每次 `session/new` 或 `session/load` 之前，Ora 以 INFO 记录 `sending ACP session configuration`，包含会话 id、Agent、provider session id、ACP 方法、选择模式、server 数量，以及选中插件的 id、包版本、配置 revision 和传输方式。命令、参数、环境变量、URL、header 和设置值都不记录。

setup 成功表示完整列表已交付并被接受，从不表示 Agent 已经连好：ACP 没有 MCP 回执。Agent 应当把 `mcpServers` 视为该会话的完整集合，并在回答之前连好它们。Ora 让多个会话共享一个 Agent 进程，而 OpenCode 到 1.18.30 为止把 ACP 提供的注册保留在进程级别，所以一个空列表可能移除不了同一进程中早先会话注册的 server（[OpenCode #32371](https://github.com/anomalyco/opencode/issues/32371)）。

## 宿主 MCP 健康

交付并不能证明任何连接，所以宿主维护第三个与交付无关的事实：**宿主 MCP 健康**，由宿主自己执行的有界 MCP 握手得出。

- **何时**：MCP 包安装并变得可用之后、配置保存为完整之后，以及每次 `session/new` 或 `session/load` 之后。用户的"重新检测"操作和对仍未知成员的回填也会触发。同一身份的并发触发共享一次探测；安装、保存和启动会话从不等待它。
- **如何**：探测使用与 setup 交付相同的已解析传输（从不再解析一次配置），执行 `initialize`、`notifications/initialized` 和 `tools/list` 后断开；stdio 探测还会确认子进程已被回收。硬超时 8 秒。客户端在 `ora-utils` 中，放在单独的 feature 后面，安装时不使用，安装仍然既不运行命令也不建立连接。
- **身份**：插件 id、精确的包版本、配置 revision 和传输方式；对替换了 `{ "context": "workspace" }` 的成员，还包括会话的绝对 cwd。插件卡片没有会话目录，所以这类成员在卡片上保持 `Unknown(context_missing)`。卸载、更新、配置变化或失去资格都会丢弃旧结果。什么都不持久化；重启之后一切都是 `Unknown(not_probed)`。
- **结果**：`Healthy`；`Unhealthy { code }`，code 为 `mcp_spawn_failed`、`mcp_exited_prematurely`、`mcp_handshake_failed`、`mcp_probe_timeout`、`mcp_tools_unavailable`、`mcp_http_unreachable`、`mcp_http_unauthorized`、`mcp_http_server_error` 之一；或 `Unknown { not_probed | context_missing }`。这组 code 与 setup 错误相互独立。

健康从不改变交付：探测失败不会让 setup 失败，不会移除成员，也从不产生部分列表。探测健康也不等于"已在会话中生效"：Agent 仍然自己去连，所以两者可能朝任一方向不一致。

展示不含机密：插件卡片在安装和配置状态旁显示健康状态，并提供重新检测；会话显示一个不阻塞的横幅，列出它自己的有效集合中 `Unhealthy` 的成员；`listMcpHealth` 回答这两种视图，`probeMcpHealth` 重新探测；`McpHealthChanged { plugin_id }` 通知客户端重新查询。在 setup 日志之后，一条结构化事件把会话和其成员的身份、code 和探测耗时对应起来。
