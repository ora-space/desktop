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

工作流会话在首次创建时把节点选择直接持久化到 Session 行，并在恢复、重建、切换 Agent 和刷新时读取该值。恢复过程不再通过节点运行关联反推权限，因此关联缺失或成为孤儿数据时，也不会把显式选择扩大为自动发现。由于尚无用户使用此处的 MCP 授权，迁移 `0011` 将所有历史会话统一设为空显式选择，不读取工作流关联或快照。因此，已有普通聊天也不会自动发现 MCP。新建普通会话由业务代码明确选择自动发现；新建工作流会话保存节点的显式选择。修改草稿不会影响已有运行的选择。

选中插件的版本和设置仍是实时输入，继续使用现有安全刷新边界。白名单以外的插件变化不会改变当前会话的 Desired 版本。编辑器开关用于配置后续运行，不用于即时修改正在运行的会话。

MCP 刷新、Skill Effect 变更和 Agent 替换共用 Agent Session Barrier，确保新请求等待安全时机。它们不共用 Effect 状态：MCP 不会变成 Effect Resource、Desired 或就绪信号。

## 诊断日志与 Agent 一致性

每次发送 ACP `session/new` 或 `session/load` 前，Ora 都会写入一条名为 `sending ACP session configuration` 的 INFO 日志。日志包含 Ora 会话 ID、Agent、已有时的提供方会话 ID、ACP 方法、选择模式、服务数量，以及所选插件的 ID、包版本、配置修订号和传输类型。命令、参数、环境变量、HTTP URL、请求头和设置值不会写入日志。

Agent 适配器应把传入的 `mcpServers` 列表视为该会话的完整集合。Ora 保留共享 Agent 进程模型，不为每个会话创建独立 OpenCode 进程。OpenCode 截至 1.18.30 仍会在进程范围保留通过 ACP 注入的 MCP 注册，因此 Ora 虽然正确发送空列表，OpenCode 仍可能向当前会话暴露同一进程中较早会话注册的服务。该 provider 一致性缺口由 [OpenCode issue #32371](https://github.com/anomalyco/opencode/issues/32371) 跟踪。

等待时长是非活跃窗口，并随投递内容放宽，因为连接投递的服务正是一个合规 setup 中最慢的部分：携带 MCP 服务的 `session/new` 或 `session/load` 最多等待 120 秒无响应（而不是 30 秒），期间 Agent 发出的 setup 通知会重置窗口（见[ACP Agent 运行时](agent-runtime.zh.md)）。

ACP 1.6.0 不为 MCP 连接提供任何回执：`NewSessionResponse` 和 `LoadSessionResponse` 没有 MCP 状态字段，`SessionUpdate` 也没有 MCP 变体，因此 setup 成功只表示完整列表已被投递并接受，永远不表示 Agent 已完成连接。协议文档中的 session-setup 时序要求 Agent *先*连接投递的服务、*再*回答；先回答、后在后台连接的 Agent，可能在连接完成前就开始处理 prompt，且不产生任何 Host 可见信号。这个窗口属于 Agent 一致性责任：Gemini CLI 曾有完全相同的竞态（[gemini-cli #18893](https://github.com/google-gemini/gemini-cli/issues/18893)），其修复方式是让 prompt 处理等待 MCP 初始化完成（[#20205](https://github.com/google-gemini/gemini-cli/pull/20205)）；Claude Code 也在 [claude-code #83555](https://github.com/anthropics/claude-code/issues/83555) 跟踪同类首轮工具竞态。在不改变投递语义的前提下，Ora 自己、只在内存中观察 Host 侧连接健康；见[运行健康](#运行健康)。

## 安全与兼容

设置值只能存在于配置存储、短暂的内存快照和发送给可信 Agent 的 ACP 消息中。不得进入 Effect、SQLite、工作区文件、日志、错误、UI DTO、版本摘要或 Agent 进程环境变量。日志只能包含上述不含秘密的版本身份信息。工作流只保存插件 ID 和开关。错误仅包含插件 ID、设置 ID、传输类型和稳定错误码。

Ora 不会为 MCP 创建、修改或删除 `.mcp.json`、OpenCode JSON/JSONC、所有权旁文件、Git 排除文件或其他工作区路径。已有用户 MCP 文件保持原样。本实现没有从未发布的文件物化方案迁移的步骤。
因此，安装 MCP 只表示把它加入全局可选目录；工作流节点的显式选择才是 Session 级授权决定。

## 运行健康

setup 成功只表示 Host 发出了完整的 `mcpServers` 列表，不表示 Agent 已连上这些 Server、已列出工具，或已让模型用到它们：ACP 1.6.0 不为 MCP 连接提供任何回执，一个跳过或连接失败却仍正常回答 `session/new` 的 Agent 也是合规的。因此 Ora 自己持有第三类事实——**Host MCP 健康**，由 Host 以 MCP 客户端身份执行一次有界握手建立。

在 MCP 包安装且合格之后、配置保存为 `Complete` 之后，以及每次 `session/new` 与 `session/load` 之后，Host 都会对「Session setup 投递的同一份绑定产物」执行一次握手：它来自构建 ACP payload 所用的同一个 `resolve_mcp_transport`，不是第二条配置解析路径。握手只执行 `initialize`、`notifications/initialized` 与 `tools/list`，随后拆除连接；stdio 探测还会关闭子进程 stdin 并确认进程已回收。硬超时为 8 秒，远低于 session setup 的预算，因此探测不能在时间上冒充 setup。

健康是独立通道，不改变投递：探测失败不会使 `session/new` 或 `session/load` 失败，不会把成员移出 Effective MCP Set，也不会产生部分 `mcpServers` 列表；分辨率失败仍按上文让整个 setup 失败。安装、保存与建立会话都不等待探测。同一身份的并发触发共享一次探测；保存只探测一次，不自行重试；其余探测来源只有用户的「重新检测」和会话建立时对仍 Unknown 成员的补探测。

健康身份是 canonical Plugin ID、精确包版本、配置 revision 与 transport。参数替换 `{ "context": "workspace" }` 的成员还绑定 Session 的绝对 `cwd`。插件卡片没有 Session 目录，因此这类成员在卡片上保持 `Unknown(context_missing)`，绝不会用伪造路径探测；会话探测结果也不会回写到卡片。卸载、升级到新版本、配置 revision 变化或失去资格后，旧身份的结果立即失效。健康不持久化，也没有 TTL 或后台复检，所以 Ora 重启后每个身份都重新从 `Unknown(not_probed)` 开始。

状态为 `Healthy`、`Unhealthy { code }` 或 `Unknown { reason }`。`reason` 只有 `not_probed` 与 `context_missing`；`code` 是 `mcp_spawn_failed`、`mcp_exited_prematurely`、`mcp_handshake_failed`、`mcp_probe_timeout`、`mcp_tools_unavailable`、`mcp_http_unreachable`、`mcp_http_unauthorized`、`mcp_http_server_error` 之一。错误码族封闭，且刻意与 setup 稳定码（`mcp_setting_missing`、`mcp_http_capability_missing` 等）分开：探测结果是运行期观察，不是投递错误。

**Host 探测成功不等于「会话内已生效」。** 它只说明 Host 在那一刻完成了握手；它不是 `MCP Ready`，不是 Active revision，也不代表 Agent 已连上 Server 或已让模型看到其工具。Agent 仍自行连接并运行 Server，因此探测结果与会话实际情况可能双向不一致。

呈现面保持脱敏，并与既有事实彼此独立：

- 插件卡片把健康作为第三行状态呈现，与安装状态、配置完整性并列，包含稳定码与「重新检测」；配置不完整或不合格的插件不被探测，也不展示该行。
- 会话内的非阻塞横幅只列出该 Session Effective MCP Set 中的 `Unhealthy` 成员（普通聊天是自动发现集合，工作流是冻结白名单），并提供进入插件配置的入口。未选择的 MCP 不会出现在这里，显式空集合不展示横幅，`Unknown` 不是失败，也从不阻塞 prompt。
- `listMcpHealth` 按可选 `cwd` 回答卡片视角（无 `cwd`）或某个会话视角（绝对 `cwd`），返回值只有身份、状态与稳定码，绝不含设置实值、凭据、argv、env、headers 或第三方响应文本。`AppEvent::McpHealthChanged { plugin_id }` 只通知前端按身份重新查询，不携带状态本体。
- 在既有 INFO 事件 `sending ACP session configuration` 之后，追加一条结构化探测结果事件，把该次 Session 与成员的「身份、稳定码、耗时」配对。它记录的是 Host 观察到的事实，不声称 Agent 已做同样的事。

用于探测的 MCP 客户端位于 `ora-utils`，不含 Ora 领域词汇，由独立 Cargo feature 门控，并刻意不进入 plugin-manager 的安装验证路径：安装阶段仍然既不执行命令，也不发起连接来决定包是否有效。
