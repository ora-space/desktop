# Agent MCP 配置交付

## 问题陈述

Ora 已能从官方插件市场发现 Agent 和 MCP 插件，下载 `.orax` 发布包，校验 SHA-256，安全解压，验证插件类型并原子安装；也能持久化用户填写的 MCP 配置。但这些供给侧能力尚未让 MCP 真正进入 Agent 对话。

当前，已安装 MCP 的声明和配置无法传递到 Agent Workspace：Ora 没有运行时 `Resolved MCP` 模型、MCP Effect Source、Workspace MCP Desired Set、Agent 原生配置物化能力，也没有覆盖 MCP 配置的目标就绪门禁。OpenCode Agent 插件只消费现有 Skill Surface，不写入 OpenCode MCP 配置。因此，即使用户成功安装 OpenCode 和 Tavily 并填写 API Key，仍无法在 Ora 对话中使用 Tavily。

现有生命周期也无法安全支撑精确版本引用：插件升级会立即删除旧版本；Agent 生命周期检测忽略“只有版本发生变化”的情况；卸载会在 Effect 引用退出前删除插件文件。对于可执行文件位于插件包内的 stdio MCP，这可能令 Desired State 或已应用状态指向已不存在的版本。

本功能必须打通“插件市场安装—配置—Workspace 收敛—对话可用”的最小闭环，同时满足以下约束：Agent 原生格式不进入 Ora Core；未完成配置的 MCP 不影响普通对话；新请求不能进入只完成部分配置的 Workspace generation；Workspace 与消费者的配对不能依赖启动时的一次性事件。

## 解决方案

所有已安装 MCP 默认具有启用意图（Default-enabled）。缺少必填配置时，MCP 保持“已安装、待配置”（Needs Configuration），但不进入任何 Workspace MCP Desired Set。配置完整并可生成 Ready 的不可变 MCP Source Revision 后，Ora 自动将该 revision 传播到全部现有 Agent Workspace；未来创建的 Workspace 也自动包含它。

深化现有 MCP 配置模块：将安装期声明与有效配置解析为目标无关的 MCP 定义，并结合具体 Workspace 生成 `Resolved MCP`。Skill 与 MCP 共享 Effect 的 generation、持久化请求、租约、操作日志、恢复和安全扫描机制，但保留各自强类型的规划器与物化适配器。

Agent 插件通过可选、带版本的 MCP Configuration Capability 声明能力。支持该能力的插件通过单一且幂等的 `agent/configureWorkspace` 方法接收 Agent Target 的完整 MCP Configuration Snapshot。目标原生配置由 Agent 插件渲染，Ora Host 不理解其格式。Agent 不支持的 transport 按 MCP、按 Agent Target 记录为非阻塞问题；支持项仍正常应用，目标进入 Ready with Issues，并允许对话。

首个适配器由 OpenCode Agent 插件实现，只物化 Streamable HTTP MCP。它在 Workspace 下生成 Ora 全量托管的 `.opencode/opencode.json`，与用户维护的项目根目录 `opencode.json`/`opencode.jsonc` 分离。Ora 将精确生成路径加入仓库本地 Git exclude，收紧文件权限，从诊断中清除配置值，并用持久化操作记录和指纹实现崩溃恢复与外部改动检测。

Agent Target 只有在当前 Workspace generation 所需的 Skill 与 MCP 操作全部处理完成后才接收新请求。普通配置变化不打断正在执行的 turn。收敛器等待相关 Session 空闲，关闭新请求入口，执行一次 idle barrier，应用 Skill 与 MCP，重启或恢复 Agent，最后推进 Agent Target Ready Generation 并重新开放入口。

端到端流程如下：

1. 用户从官方插件市场安装 OpenCode Agent 插件与 Tavily MCP。
2. Tavily 显示为“已安装、待配置”；它不进入 Desired Set，现有对话不受影响。
3. 用户在 Ora 配置界面填写并保存 Tavily API Key。
4. Ora 发布 Ready 的 Tavily MCP Source Revision，并传播到所有现有 Agent Workspace。
5. Effect 收敛器等待各 OpenCode Agent Target 空闲，发送完整快照，并记录文档和条目回执。
6. OpenCode 插件原子写入 Workspace 托管文档，并通过协商的协调协议刷新运行时。
7. Ora 推进 Ready Generation，允许新 turn 进入。
8. OpenCode 从 Workspace 配置发现 Tavily，并在对话中暴露其 MCP 工具。
9. 此后创建的 Workspace 无需用户再次选择或点击“应用”，即可自动获得 Tavily。

## 用户故事

1. 作为 Ora 用户，我希望从官方插件市场安装 Agent 插件，以便在 Ora 中使用该 Agent。
2. 作为 Ora 用户，我希望从官方插件市场安装 MCP，以便由 Ora 校验和管理 MCP 定义。
3. 作为 Ora 用户，我希望缺少必填项的 MCP 明确显示 Needs Configuration，以免把安装成功误认为运行时可用。
4. 作为 Ora 用户，我希望未配置完成的 MCP 不影响现有对话，以免安装行为使全部 Workspace 不可用。
5. 作为 Ora 用户，我希望安装未配置完成的 MCP 后立即进入明确的配置入口。
6. 作为 Ora 用户，我希望保存完整配置后 MCP 自动生效，无需再次点击“应用”。
7. 作为 Ora 用户，我希望 Ready 的 Default-enabled MCP 自动传播到全部现有 Workspace。
8. 作为 Ora 用户，我希望未来创建的 Workspace 自动获得全部 Ready MCP，使创建顺序不影响能力。
9. 作为 Ora 用户，我希望重置或丢失必填配置后，MCP 自动退出 Workspace Desired State。
10. 作为 Ora 用户，我希望重新补齐配置后，MCP 自动重新加入全部 Workspace。
11. 作为 Ora 用户，我希望配置变化等待正在执行的 turn 结束，避免普通配置修改中断工作。
12. 作为 Ora 用户，我希望 Agent Target 应用新 generation 时阻止新 turn 进入，避免能力状态新旧混杂。
13. 作为 Ora 用户，我希望等待门禁期间看到明确的等待或应用状态。
14. 作为 Ora 用户，我希望能够取消尚未通过门禁的请求。
15. 作为 Ora 用户，我希望阻塞性收敛失败提供稳定且可执行的错误信息。
16. 作为 Ora 用户，我希望 Agent 不支持某个 MCP transport 时仍能正常使用其他能力。
17. 作为 Ora 用户，我希望“不受支持”按 Agent Target 独立记录，使同一 MCP 可在不同 Agent 上产生不同结果。
18. 作为 Ora 用户，我希望 Agent 插件升级后自动重新评估 transport 支持。
19. 作为 Ora 用户，我希望同一 generation 的 Skill 和 MCP 一起就绪后才开始新 turn。
20. 作为 Ora 用户，我希望 Ora 保留我维护的项目根目录 OpenCode 配置。
21. 作为 Ora 用户，我希望 Ora 生成的 MCP 配置位于独立托管文档中，使所有权和清理行为可预测。
22. 作为 Ora 用户，我希望 Ora 拒绝接管托管路径上既有的非托管文件。
23. 作为 Ora 用户，我希望 Ora 检测托管文档被外部修改的情况，避免无法证明所有权时覆盖内容。
24. 作为 Ora 用户，我希望冲突恢复先备份再重建，不允许无备份强制覆盖。
25. 作为 Ora 用户，我希望生成文件只加入仓库本地 Git exclude，不修改项目 `.gitignore`。
26. 作为 Ora 用户，我希望托管路径已被 Git 跟踪时阻止物化。
27. 作为 Ora 用户，我希望生成文件采用仅当前 OS 用户可访问的限制性权限。
28. 作为 Ora 用户，我希望配置值不进入日志、审计、公开错误或遥测。
29. 作为 Ora 用户，我希望卸载先清理 Agent 配置，再删除 MCP 插件包。
30. 作为 Ora 用户，我希望“保留数据”卸载后重装可以复用兼容配置。
31. 作为 Ora 用户，我希望“删除数据”仅在目标配置清理完成后删除配置数据。
32. 作为 Ora 用户，我希望 Ora 重启后能继续中断的卸载操作。
33. 作为 Ora 用户，我希望升级先完整验证新包，失败时旧 revision 继续可用。
34. 作为 Ora 用户，我希望精确版本仍被引用时保留旧插件包。
35. 作为 Ora 用户，我希望新版本变为 Needs Configuration 时退出旧有效版本，而不是暗中继续运行旧版本。
36. 作为 Ora 用户，我希望升级和卸载在等待收敛期间显示为进行中，不提前报告完成。
37. 作为 Agent 插件作者，我希望 MCP 支持是可选且带版本的能力，不影响基础对话契约。
38. 作为 Agent 插件作者，我希望明确声明支持的 transport，使 Ora 可在渲染前完成分类。
39. 作为 Agent 插件作者，我希望收到完整快照而非增删改事件，以便丢失通知或重启后仍可幂等收敛。
40. 作为 Agent 插件作者，我希望 Ora 提供稳定托管标识、精确包版本和规范化 transport，无需读取其他插件内部状态。
41. 作为 Agent 插件作者，我希望目标格式渲染留在插件内部，避免 Ora Core 依赖具体 Agent schema。
42. 作为 Agent 插件作者，我希望配置调用携带 Workspace root、generation 和 operation identity，以实现幂等写入与可审计回执。
43. 作为 Agent 插件作者，我希望畸形 Host 请求和不支持的协议版本在进入适配器前被拒绝。
44. 作为 Agent 插件作者，我希望 Host 严格校验回执，避免不完整结果被误标为 Ready。
45. 作为 MCP 插件作者，我希望安装有效性、配置完整度、Source 身份和精确版本相互独立且可复现。
46. 作为插件市场维护者，我希望 Host 依赖和发布元数据受到强制校验。
47. 作为开发者，我希望 Skill 与 MCP 复用一套可恢复 Effect 机制，同时保留强类型规划器和高层测试接缝。
48. 作为支持或安全审查人员，我希望错误安全稳定、明文范围明确，并拒绝未来 Secret Setting 误走普通字符串链路。

## 实现决策

### 1. 权威术语与不变量

- 以项目上下文定义的 Agent Workspace、MCP Package、Default-enabled MCP、Workspace MCP Desired Set、MCP Source、MCP Source Revision、Agent Target、Agent Target Ready Generation、MCP Configuration Capability、MCP Configuration Snapshot、MCP Materialization、Managed Agent Configuration Document、Ready with Issues 和 Needs Configuration 为权威术语。
- 安装有效、配置完整、Source Ready、Workspace Desired、目标已应用和运行时工具可用是六个独立事实，禁止依据其中一个推断另一个。
- 所有已安装 MCP 均具有 Default-enabled 意图；只有 Ready MCP 存在活跃 Source Revision 并进入 Desired Set。
- 本期 MCP 选择范围为 Workspace，且自动完成；不提供 Workspace 级关闭、Agent 级手工选择或 Session 级选择。
- Agent 原生字段只能出现在 MCP Materialization 接缝之后；Host 领域类型禁止包含 OpenCode 字段或渲染规则。
- Agent Target 仅在所需 generation 达到 Current 或 Ready with Issues 时允许新 turn；Waiting、Applying、Degraded 和 Recovery Required 均不可放行。
- Unsupported by Agent 是目标级非阻塞状态；Needs Configuration 是插件级状态并阻止 Desired membership，二者不得混用。
- 必须通过持续收敛同时实现“新消费者→既有 Workspace”和“新 Workspace→既有消费者”。所有消费者声明进入收敛器读取的唯一 Agent Effect declaration snapshot；禁止新增 MCP 专用声明注册表。

### 2. MCP 配置与解析模块

- 深化现有 MCP 配置模块，不在 Backend 调用方拼装解析，也不增加只做转发的薄层。
- 模块输入包括已校验 MCP descriptor、canonical Plugin identity、精确 package version 与 package root、配置存储快照；需要 Workspace 解析时再传入 Agent Workspace root。
- 模块负责声明编译、完整度判断、普通 Setting 有效值、前后缀展开、HTTP header 校验、stdio 参数渲染、包内可执行文件 containment、Workspace 上下文解析、规范化摘要，以及目标无关 MCP 定义与 `Resolved MCP` 构造。
- 返回值必须是穷尽枚举：Ready（携带不可变 Source candidate）、Needs Configuration（缺失或无效 Setting ID）、Unavailable（稳定安全原因）或精确版本/路径 containment 错误；调用方不得检查可选字段来猜测状态。
- Source Revision 身份绑定 canonical Plugin identity、精确 package version、descriptor digest、configuration-store revision、normalized resolved-state digest 和 payload schema version，不能只使用 SemVer。
- revision payload 包含规范化 transport 与当前声明为普通 string/number/boolean 的有效值，不包含 Workspace path；Workspace 上下文在构造目标快照时解析。
- Tavily 当前把 API Key 声明为普通 string，本期明确按非 Secret 的本地明文数据处理。该值可存在于插件配置存储、不可变 revision payload、内存/JSON-RPC 快照、恢复所需的持久化 operation payload 和托管文档中，但全部仅限当前 OS 用户本地环境。
- 普通 Setting 值禁止进入日志、trace、审计、公开错误、UI analytics、崩溃摘要、用户可见 digest 或 Issue。规范 digest 使用规范字节的单向 SHA-256，不能替代明文泄漏控制。
- 未来声明为 Secret 的 Setting 必须以 `mcp_secret_unsupported` 拒绝发布和 protocol v1 传输；SecretRef 需要独立协议，禁止静默降级为普通字符串。
- stdio 可执行文件必须位于被保留的精确插件版本内，并在使用前再次校验；工作目录始终是 Agent Workspace，不是共享 Agent 进程的中性目录。

### 3. Source 发布与自动传播

- 复用通用 Effect source 存储，使用 `effect_kind = mcp`、`source_kind = plugin`。稳定 source key 为 canonical Plugin identity；包或配置变化只产生同一 Source 下的新 revision。
- Workspace Desired State 增加强类型 MCP map；Skill map 与 MCP map 保持独立字段，不合并为通用可选字段 payload。
- 安装成功始终更新已安装插件目录和配置摘要；仅当配置 Complete 且解析为 Ready 时发布活跃 Source。
- 保存、重置、恢复或重新校验配置后，在配置存储写入成功后执行一次 readiness synchronization。
- Ready 转换必须原子完成：发布不可变 revision、推进 source head、更新全部既有 Workspace desired item、推进 generation，并 upsert 对应 Agent Target 的持久化 reconcile request。
- Ready→Ready 更新保持 Source identity，只推进 revision。Ready→Needs Configuration/Unavailable 退休活跃 Source、移除全部 Desired item 并触发清理，但保留 Default-enabled 意图。重新 Ready 后自动加入全部 Workspace。
- 新 Workspace 初始化在同一数据库事务中选取全部活跃 Skill head 与 Ready MCP head，不选 retired、unavailable 或配置不完整的 MCP。
- Desired 变化与 durable request 同事务提交；内存通知只是优化。配置存储与 SQLite 无法共享事务，因此启动修复和周期安全扫描必须比较安装包、配置 revision 与活跃 Source，修复缺失发布、ghost Source 和缺失请求；相同输入不得重复推进 generation。

### 4. 共享 Effect 收敛与 Agent Target 调度

#### 持久化模型

- 扩展通用 Source 约束以接受 `mcp`；增加 `mcp_source_revision_metadata`，以 source revision 为键，保存 canonical Plugin identity、精确包版本、descriptor digest、configuration-store revision、resolved-state digest 与 payload schema version。垃圾回收只查询这些索引列，不解析不透明 JSON。
- 增加 `effect_agent_targets`（Workspace identity + Agent Plugin identity 唯一），保存 capability revision 与生命周期；增加 `effect_agent_target_status`，保存 desired/observed/applied/ready generation、phase、status version 和时间戳。
- 将 `effect_reconcile_requests` 改为 Agent Target 维度，保留 requested generation、wake reason、lease、attempt 和调度字段。迁移时，同一目标的既有 surface request 合并为最大 generation 与最早到期时间。
- 保留物理 Skill Surface；新增逻辑 MCP Surface，adapter kind 为 `agent_configuration`、format kind 为 `mcp_configuration.v1`，consumer 指向 Agent Plugin，不保存 Host 渲染的文件路径。
- `effect_conditions` 改由 Agent Target 拥有，可选关联 surface/consumer，并强制 `impact` 为 `blocking` 或 `non_blocking`；既有条件迁移为 Blocking。
- 增加 `effect_managed_documents`（Agent Target + materialization kind 唯一），保存 locator、applied fingerprint、applied generation 与 status revision；增加 `effect_managed_document_entries`，分别约束“document + managed identity”和“document + native key”唯一，每项引用精确 MCP source revision。
- 增加 `effect_operation_source_revisions`，使恢复与垃圾回收无需解析 operation payload 即可查询精确版本引用。
- 增加 `plugin_package_operations`，记录 Plugin identity、`update`/`uninstall`、previous/candidate version、phase、data-retention policy、last safe error 和时间戳；每个 Plugin identity 最多一个非终态操作。
- 外键与 check constraint 必须强制 generation 顺序、枚举合法性、文档条目所有权和终态 operation 不可变。使用单向迁移与确定性回填，不保留兼容 view 或双写期。

#### 调度与状态机

- Skill 与 MCP 共用一个 Workspace generation、request store、claim/lease 协议、operation journal、重试模型、启动恢复和周期安全扫描，但保留各自强类型 planner/adapter。
- request 以 Agent Target 为键，一次请求覆盖该目标消费的全部 Skill Surface 和当前 generation 的完整 MCP 文档投影。共享物理 Skill Surface 继续用独立 lease 和 active-operation 唯一约束串行化。
- worker claim 后、获取 admission barrier 前各读取一次最新 generation；为某 generation 创建的 Prepared operation 不得改变其 generation。
- 任一相关 Session 为 Working 或 Stopping Turn 时，目标进入 Waiting for Idle，订阅 Session 状态变化并释放 claim，不修改目标，也不定时热轮询。
- 全部 Session 空闲后，收敛器以同一 Agent Target gate 原子关闭新 turn，调用一次 idle barrier，应用所需 Skill/MCP，调用一次 restart/resume，重新开放入口并推进 Ready Generation。
- 请求 admission 与 reconcile 使用同一 keyed gate：请求先获得 gate 时收敛等待；收敛先获得 gate 时请求等待 readiness，且不得长期持有数据库事务或全局锁。
- 等待请求可取消；只在目标达到所需 generation 的 Current/Ready with Issues 后继续。进入 blocking Degraded/Recovery Required 时，以稳定结构化错误失败。
- 收敛期间出现更高 generation 时，当前不可变 operation 先完成或恢复，随后处理新 generation；旧完成结果不得清除更高请求。
- Applied Generation 表示全部目标修改完成；Ready Generation 还要求 Agent 成功恢复且 admission 已开放。Skill 与 MCP 可分别执行，但只有二者全部处理完才能推进 Ready Generation。
- condition impact 使用 Blocking/NonBlocking 枚举。Unsupported by Agent 为 NonBlocking；所有权或漂移冲突、无效插件回执、不安全路径、权限错误、Git 已跟踪冲突和 Recovery Required 均为 Blocking。
- Ready with Issues 要求全部受支持操作已 current，且只剩至少一个 NonBlocking condition。共享 Effect worker 保持有界、level-triggered；禁止创建 MCP 专用 worker 或 request 表。

| 起始状态                    | 触发条件                                  | 目标状态          | 必须执行的效果                               |
| --------------------------- | ----------------------------------------- | ----------------- | -------------------------------------------- |
| Current / Ready with Issues | Desired 或 capability revision 推进       | Pending           | upsert target request，暂不关闭 Session      |
| Pending                     | 相关 Session 为 Working/Stopping Turn     | Waiting for Idle  | 等待 Session 变化，不修改目标、不轮询        |
| Pending / Waiting for Idle  | Session 全部 Idle 且 admission 关闭       | Quiescing         | 在同一 gate 排队或拒绝新 turn                |
| Quiescing                   | idle barrier 成功                         | Applying          | 先持久化不可变 operation plan，再执行副作用  |
| Applying                    | 所需 Skill/MCP 均成功或只剩非阻塞不支持项 | Resuming          | 持久化回执、问题与 applied generation        |
| Applying / Resuming         | 可重试失败                                | Degraded          | 保留 Ready Generation，按持久化 backoff 重试 |
| Applying / Recovery         | 无法证明所有权或检测到漂移                | Recovery Required | 停止自动修改，等待备份并重建                 |
| Resuming                    | Agent 恢复且无问题                        | Current           | 推进 Ready Generation 并开放 admission       |
| Resuming                    | Agent 恢复且只剩非阻塞问题                | Ready with Issues | 推进 Ready Generation 并暴露问题             |

### 5. Agent MCP Configuration Capability 与协议

- Agent registration 增加可选顶层 `mcpConfiguration`。protocol v1 声明正整数版本、非空且不重复的 transport 集合与 coordination mode；支持 `http`、`stdio` 和 `wait_for_idle_and_restart`。OpenCode 0.3.0 只声明 `http`。
- 声明 capability 时必须同时注册 `agent/configureWorkspace`。能力或方法缺一、未知字段、重复/未知 transport、协议版本不支持均使 MCP capability 无效，但不使基础 Agent contract 无效；没有既有托管冲突时普通对话仍可用。
- 该能力是加法式扩展：旧插件省略后视为不支持全部 MCP；旧 Host 忽略未知顶层 registration 字段和额外方法，因此不提升全局 `pluginApi` major version。
- SDK 用一个可选 MCP configuration definition 同时注册 capability 与方法，避免作者通过高层 API 只声明一侧。
- `agent/configureWorkspace` 只接收完整快照。请求包含 protocol version、稳定 operation identity、稳定 Agent Target identity、绝对 Workspace root、非负 Workspace generation，以及该目标支持的完整 `Resolved MCP` 列表。
- 每个 `Resolved MCP` 包含 canonical MCP identity、stable managed identity、精确 package version 和且仅一个规范化 transport。HTTP 包含绝对 URL 与已校验 headers；stdio 包含精确 package executable、渲染后参数、已校验环境绑定和 Workspace working directory。
- 请求禁止携带原始 package manifest、原始 `assets/config.json`、完整 Settings store、Ora 数据库路径或其他插件路径。
- capability 不支持的 transport 不进入插件请求，由 Host 记录 NonBlocking Unsupported by Agent；完整快照中不再出现的既有托管项必须被插件移除。
- 插件规划整份目标文档、检查所有权与冲突、原子写入或删除，并在字节达到正常进程崩溃耐久要求后返回。
- 成功回执包含 exact applied generation、document locator、document SHA-256 fingerprint 和完整 entry receipts；每项包含 managed identity、native key、entry fingerprint 和 exact source revision identity。
- Host 必须拒绝 generation 不匹配、locator 越界、managed identity 缺失/重复、额外条目、fingerprint 非法，或未恰好覆盖全部受支持 Desired MCP 的回执。
- Waiting for Idle 只能通过修改前的既有协调接口返回，不能作为 configure 的部分成功。渲染阶段发现的 target-specific unsupported issue 必须指向请求中的 managed identity 并使用允许的稳定 code；其他失败均为 Blocking，且不得破坏上一版已提交文档。
- JSON-RPC、插件 stderr、Host trace 与 timeout error 禁止包含 header value、Setting 派生参数、environment value 或文档内容。

### 6. Agent Effect 声明收敛

- 唯一 Agent Effect declaration snapshot 同时包含 Skill Surface declarations、可选 MCP capability 与 Agent Capability Revision。
- Agent 插件注册或声明变化时，对全部现有 Workspace 收敛并唤醒目标；新 Workspace 通过 worker 常规 pass 收敛全部既有声明，禁止只在进程启动时完成反向配对。
- 生命周期变化检测必须包含精确已安装版本与 Effect declarations 的 canonical digest；只有版本或 capability digest 变化也属于真实变化。
- 插件临时停止或断连只把目标标为 unavailable 并保留 Desired；卸载通过 durable lifecycle operation 退休声明。能力恢复或新增 transport 时唤醒全部目标，并在下次完整快照后清除过期 unsupported issue。

### 7. 托管状态、操作与崩溃恢复

- Agent 原生文档由 managed-document ledger 整体拥有；多个 MCP entry 可共享同一 locator，不能复用“每个 Skill item 对应唯一文件系统目标”的不变量。
- 调用插件前持久化 Prepared operation，包含 generation、source revision identities、managed identities、计划删除项、adapter kind、payload version，以及足以在崩溃后精确重放的规范化快照。
- protocol v1 的 operation payload 可包含普通 Setting 明文，但必须遵守与配置存储和托管文档相同的本地权限与脱敏要求。
- 插件返回后，先持久化 observed document/entry fingerprints 并标记 Applied，再原子更新 document ledger、entries、conditions 和 applied generation，最后终结 operation。
- 调用前崩溃时重试同一 Prepared operation。调用中或调用后崩溃时，以同一 operation identity 与 snapshot 重放；插件读取现有文档，若 desired bytes 已存在则返回同一回执。
- 插件已写入但 ledger 未提交时，根据 durable operation 与现场指纹恢复，绝不只凭路径推断所有权。现场字节既不匹配上一版 applied fingerprint，也不匹配可恢复 Prepared 结果时进入 Recovery Required，禁止覆盖或删除。
- Recovery Required 只能手工恢复：先把冲突文档备份到带本地时间戳的文件，审计只记录 locator/fingerprint，再重跑完整 Desired snapshot；禁止跳过备份的强制覆盖。
- 受支持 MCP 集合为空时，只在 fingerprint 与 ledger 一致时删除托管文档并返回空回执；没有文档则 no-op。路径存在但没有 ledger 或 active recoverable operation 时报告 ownership conflict，禁止按名称、位置或内容相似度接管。
- 可重试 I/O 与插件传输失败使用持久化指数 backoff + jitter；Unsupported by Agent 等待 Desired/capability revision 变化；Recovery Required 等待显式恢复动作。

### 8. OpenCode 适配器

- protocol v1 只支持 Streamable HTTP；Ready stdio MCP 对 OpenCode 产生 NonBlocking Unsupported by Agent。
- 托管文档固定为 Workspace 下 `.opencode/opencode.json`，必须使用路径 API 解析。输出为 UTF-8 严格 JSON、两空格缩进、LF、确定性 key 顺序和一个末尾换行；整文件 SHA-256 为 document fingerprint。
- 文档只包含 OpenCode schema 标识和顶层 MCP map，不包含其他 OpenCode 设置。HTTP entry 使用 remote transport、解析后的 HTTPS URL、`enabled = true`、`oauth = false` 和完整 headers。
- native MCP key 在包版本和配置 revision 间稳定：对 lowercase canonical Plugin identity，将 `[a-z0-9_-]` 之外的连续字符替换为一个下划线，去除首尾下划线，将可读部分截断到 48 字符，前置 `ora_`，再追加下划线与原始 canonical identity 的 SHA-256 前 12 位小写十六进制。无字符替换时也必须追加 digest。
- key 生成必须为纯函数并覆盖碰撞测试；若仍碰撞，返回 `mcp_native_key_collision`，禁止覆盖。回执保存 canonical identity 到 native key 的映射；包版本变化不得改变 key。
- 项目根目录 `opencode.json`/`opencode.jsonc` 始终归用户所有。写入前检查 Workspace 可达的 OpenCode 配置层；用户配置已占用计划 native key 时，以 Preserved State conflict 阻止物化，不依赖配置优先级覆盖。
- 禁止使用进程级 `OPENCODE_CONFIG` 选择 Workspace，因为多个 Workspace Session 可能共享一个 OpenCode CLI 进程。
- 通过 Git 解析出的仓库本地 exclude 文件加入精确托管路径；不修改 `.gitignore`，不忽略整个 `.opencode` 或 Skill Surface。路径已被跟踪、无法检查 Git 状态或无法写 exclude 均为 Blocking。非 Git Workspace 跳过 exclude，但仍强制权限与脱敏。
- 创建和替换文件采用与插件配置数据一致的 OS 用户限制权限；权限设置失败时不得留下新明文文件。使用同目录 staging + atomic replace，并通过项目既有耐久抽象完成 fsync 等操作后再返回成功。
- 删除时只在指纹验证通过后删除托管文档，不删除 `.opencode`、Skill、用户配置或其他文件。

### 9. 配置变化、请求门禁与 UI

- MCP 安装在包安装和目录刷新成功后即返回成功，即使状态为 Needs Configuration。此时自动打开该插件配置编辑器并聚焦第一个缺失必填项。
- 保存完整配置后立即返回新 configuration revision，通过 Effect status 展示传播与目标收敛；不提供独立 Apply 按钮。
- UI 必须区分 Installed、Needs Configuration、Ready but reconciling、Current、Ready with Issues、Waiting for Idle、Degraded 和 Recovery Required。
- Ready with Issues 只显示 Agent-specific MCP identity 与安全稳定 issue code。新 turn 等待 readiness 时显示能力配置进度并允许取消，禁止把请求发送到旧 generation。
- Working turn 期间重置或使配置无效，只将新 generation 标记 pending，临时保留当前托管文档；UI 明示清理将在 turn 结束后执行。普通重置禁止取消 turn。
- Recovery Required 显示冲突 locator、previous/observed fingerprint 和备份重建动作，不显示文档内容。升级与卸载等待目标收敛时持续显示进行中。

### 10. MCP 升级与垃圾回收

- 新版本完成校验后与旧版本并存提交；安装请求不得立即删除旧版本。durable update operation 记录 Plugin identity、previous/candidate version 与 phase，启动时恢复未完成操作。
- candidate Source 发布前必须确定性完成下载、checksum、解压、kind 校验、descriptor 编译与配置 readiness。发布前失败时，旧 source head、Desired revision 与旧包继续可用。
- candidate 为 Ready 时发布新不可变 revision 并传播；为 Needs Configuration 时退休 active source、移除旧有效 MCP，但保留 Default-enabled 意图。
- Source 发布后物化失败属于普通 reconcile failure，不静默回滚 Desired；插件原子写保证新文档成功前保留上一版已提交文档。
- candidate package commit 成功后，已安装目录显示 candidate version；Effect status 单独展示 Ready、传播与应用状态，目录版本不能代表目标已就绪。
- 只有 source head、Workspace desired item、managed receipt、未终结 Effect operation、active Agent resource grant 与 durable package lifecycle operation 均不引用某版本时，才可垃圾回收。
- 垃圾回收可重试且幂等；删除无引用旧版本失败不回滚 active revision。启动扫描恢复并存版本回收；MCP 资源引用以 active source 与 lifecycle operation 为准，禁止简单选择磁盘最高版本。

### 11. MCP 卸载生命周期

- 卸载为 durable saga。首个数据库事务记录 intent、退休 active source、从全部 Desired Set 移除、推进 generation 并 upsert reconcile request。
- 清理 Agent 配置期间保留插件包与可选数据，以保证 stdio executable 与精确 source 可恢复。只有全部受影响目标处理完 removal generation 且无 managed receipt 引用后，才可删除包。
- Ready with Issues 只在被卸载 MCP 已无回执时算完成；Degraded/Recovery Required 阻止物理删除。存在文档冲突时必须先备份并重建。
- 目标清理后，以原子 staging 删除 package root 并更新内存目录。Retain Data 保留版本无关配置；重装时只有配置 Complete 且兼容才自动发布。Delete Data 在目标清理后才 staging 并删除配置根，且不得存在 Ready Source 或 recoverable operation 引用。
- 启动时恢复全部非终态 uninstall phase，对照磁盘与 Source 修复 ghost source 与缺失发布，并报告无法恢复的 staging conflict。物理删除失败只计划重试，不重新创建 Desired 或 managed entry。

### 12. Host 版本与发布

- 保持 marketplace resolver schema version 1，不向 release manifest 添加当前不支持的 `pluginApi`、`contractVersion` 或 `engines`。
- 在 marketplace install、local import、update preparation 与 startup discovery 强制执行既有 Ora dependency 版本约束，不能只校验语法。
- 版本匹配通过单一注入式 host-version provider 读取真实 Desktop 产品版本，禁止使用 workspace crate 的 `0.0.0`。
- 不兼容包在安装或激活前以 `plugin_host_version_incompatible` 拒绝，并只携带有界的 actual/required 参数。
- MCP capability 独立协商 protocol v1，不提升全局 plugin protocol major。
- 统一 SDK 各 manifest 的版本元数据，发布不低于 `0.5.0` 的 SDK；先发布 Ora Host，再发布 OpenCode Agent `0.3.0`。OpenCode release archive 可下载且校验通过后才更新 marketplace；市场 manifest 与包内 manifest 的 identity/kind/version 必须一致，checksum 必须匹配下载字节。
- Tavily `0.1.0` 作为验收 MCP 保持不变，除非发现与本功能无关的独立打包缺陷。

### 13. 稳定错误与安全诊断

- 公开和持久化失败使用稳定 code，message 只用于解释，禁止参与控制流。
- 必须覆盖：`mcp_needs_configuration`、`mcp_configuration_unavailable`、`mcp_source_unavailable`、`mcp_unsupported_by_agent`、`mcp_secret_unsupported`、`mcp_capability_invalid`、`mcp_capability_version_unsupported`、`mcp_configuration_failed`、`mcp_configuration_response_invalid`、`mcp_materialization_conflict`、`mcp_native_key_collision`、`mcp_config_file_tracked`、`mcp_config_git_exclude_failed`、`mcp_config_permissions_failed`、`mcp_package_version_referenced`、`mcp_uninstall_waiting_for_targets` 和 `plugin_host_version_incompatible`。
- error parameter 只允许 canonical Plugin identity、Agent Plugin identity、Workspace identity、generation、transport kind、Workspace 相对 locator、required/actual host version 与 fingerprint。
- 禁止包含带凭据 URL、header/environment value、Setting 派生参数、原始配置 JSON、文档字节或向前端暴露绝对 package data path。
- 结构化日志使用 Ora logging wrapper 与本地时间；即使 trace 级别也不记录配置 payload 或 JSON-RPC body。审计只记录 identity、operation phase、generation、locator、fingerprint、outcome code 与时间戳。

### 14. 文档更新

- 更新现行 MCP 插件规范，以 Workspace Effect 驱动的 `agent/configureWorkspace` 替代启动前 `configure_agent`。
- 更新 Agent 规范，说明可选 capability、共享进程约束、Agent Capability Revision 与 admission gate。
- 更新 Effect declaration/watcher 规范，说明 MCP readiness、target-keyed request、Skill/MCP 联合 readiness、condition impact、Ready with Issues 与双向收敛。
- 模块职责、接口、生命周期或失败语义变化时，同步更新对应英文 README；MCP 配置、Effect、Backend Agent Runtime、插件 Runtime/生命周期/管理器、数据库 repository 与 SDK 文档必须与实现一致。
- 项目 glossary 与已接受 ADR 为权威；研究文档只提供证据，不是规范来源。

### 15. 交付顺序

1. 落地域类型、数据库迁移、MCP 解析、Source 发布、启动修复及模块测试。
2. 将 Effect 调度重构为 Agent Target request 与联合 readiness，同时保持现有 Skill 行为。
3. 落地可选 registration capability、Host 协议适配器、SDK 接口与兼容性测试。
4. 落地 OpenCode 托管文档、Git/权限保护、UI 状态与 Backend 集成测试。
5. 落地引用感知的升级、卸载、恢复与 Host 版本约束。
6. 更新规范与 README，运行完整仓库检查，依次发布 SDK、OpenCode、marketplace，并完成 Tavily 手工验收。

## 测试决策

### 测试原则与接缝

- 最高价值测试接缝是通过 Backend/Application 接口执行一次完整 Workspace Effect reconcile：使用真实临时 SQLite、临时 Workspace、伪 Agent Plugin Runtime adapter 与真实文件系统 adapter，贯穿 Desired 传播、持久化调度、Agent 协调、配置调用、回执、readiness 与恢复。
- 断言外部可观察状态：Workspace generation、Agent Target phase/Ready Generation、完整文档字节与 fingerprint、协议接缝上的插件调用、包版本保留、公开状态与稳定错误码；不绑定私有 helper 调用顺序。
- MCP resolver 另做聚焦模块测试，覆盖表达式渲染、完整度、URL/header、package containment、canonical digest 与 Workspace context。插件 wire protocol 做兼容性测试；OpenCode adapter 在临时 Git/非 Git Workspace 中做契约测试，并优先比较完整对象。
- 扩展既有 Effect worker/recovery、配置服务、安装升级、Backend bootstrap、SDK 与 UI 测试接缝，不为测试增加产品公共接口。

### 必须实现的自动化场景

1. 缺少必填 Setting 的静态有效 MCP 安装成功并显示 Needs Configuration。
2. Needs Configuration 不创建 active source、Desired item、generation bump 或 configure call。
3. 保存完整配置后发布一个 Ready revision 并传播到全部既有 Workspace。
4. 新 Workspace 在事务初始化时获得全部 active Ready MCP。
5. 相同有效配置 no-op 不产生 revision、generation 或 request。
6. 有效值变化在稳定 Source 下发布新 revision 并推进全部 Desired。
7. 重置必填项退休 Source、移除 Desired，并最终清理 Agent entry。
8. 补齐配置后自动恢复同一 Source identity 并重新传播。
9. 配置存储写入后、Source 发布前崩溃可由启动修复。
10. Source 发布后、内存通知前崩溃可由 durable scan 修复。
11. 新 capability 收敛到全部既有 Workspace。
12. 新 Workspace 经 worker 收敛全部声明，无需插件再次启动。
13. 只有插件版本或 capability digest 变化也会唤醒目标。
14. capability 缺失产生 NonBlocking Unsupported，基础对话仍可用。
15. OpenCode 应用 HTTP、跳过 stdio，并进入 Ready with Issues。
16. 同一 MCP 在 transport 支持不同的 Agent 上产生独立结果。
17. 插件新增 transport 后清除旧 unsupported issue 并自动物化。
18. 重复、畸形 capability 不使基础 Agent contract 失效。
19. 完整请求/回执 round-trip 保留允许字段并拒绝未知版本。
20. 回执缺失、重复、额外或不匹配时不推进 Ready Generation。
21. 快速连续 generation 合并到最新请求，但不修改已 Prepared operation。
22. Working Session 进入 Waiting for Idle，不修改目标、不热轮询。
23. 最后一个 Session 变 Idle 后唤醒目标，并在修改前取得 admission。
24. idle check 与 quiescing 之间不能有新 turn 穿过 gate。
25. 等待请求在 Current/Ready with Issues 后继续，等待期间可取消。
26. blocking Degraded/Recovery Required 以稳定安全错误终止等待请求。
27. 同一 generation 的 Skill 与 MCP 均完成后才推进 Ready Generation。
28. Skill no-op + MCP 变化不重复写 Skill。
29. MCP no-op + Skill 变化不重复调用 target configure。
30. reconcile 期间到达的第二个 generation 保持 pending。
31. OpenCode 将 Tavily 渲染为启用、非 OAuth 的 remote MCP，并使用已解析 Authorization header。
32. native key 确定、稳定、限长、字符安全并检测碰撞。
33. OpenCode 输出满足 schema、排序、缩进、换行与 fingerprint 约定。
34. 托管路径不覆盖项目根 `opencode.json`/`opencode.jsonc`。
35. 用户配置占用 native key 时产生 Preserved State conflict。
36. 既有非托管 `.opencode/opencode.json` 不被修改。
37. 外部修改托管文档时进入 Recovery Required，不覆盖。
38. 备份重建先保留冲突字节，再生成 Desired 文档。
39. Git Workspace 只增加精确 repository-local exclude。
40. 托管路径已跟踪时，即使存在 exclude 也阻止物化。
41. 非 Git Workspace 跳过 exclude，仍应用限制权限。
42. Git exclude 或权限失败时不留下新明文文档。
43. 删除最后一个 MCP 时只删除 fingerprint 匹配的托管文档。
44. 插件调用前崩溃重试同一 Prepared operation。
45. 插件写入后 ledger 提交前崩溃可安全恢复，且不按路径推断所有权。
46. ledger 提交后 operation finalize 前崩溃可幂等完成。
47. 两个 worker 不能同时 claim 同一 Agent Target。
48. 多目标共享物理 Skill Surface 时仍串行修改。
49. 可重试插件/文件错误持久化 backoff，日志不含配置值。
50. MCP 升级保留旧版本直至全部引用消失。
51. candidate 校验失败时旧 Source/Desired 保持 current。
52. Ready candidate 发布精确 revision；物化失败后不暗中回滚。
53. Needs Configuration candidate 移除旧有效 MCP 并保留启用意图。
54. Desired、receipt、operation、runtime grant 或 lifecycle 引用阻止垃圾回收。
55. 卸载先退休 Desired 并清理 Agent 配置，再 staging 删除包。
56. Retain Data 卸载重装后复用兼容配置。
57. Delete Data 只在目标清理后删除配置。
58. Recovery Required 阻止物理卸载，直至备份重建完成。
59. 启动恢复中断的 update、uninstall 与 garbage collection。
60. 启动退休 absent package 的 ghost source，并为已安装 Ready package 补发 Source。
61. Host 版本约束在 install、update、local import 均生效。
62. 版本匹配使用注入的 Desktop 产品版本。
63. marketplace 与 OpenCode release 的 identity、kind、version、URL、checksum 完全匹配。
64. UI 区分安装成功与 Needs Configuration，并提供编辑器。
65. 保存完整配置后无需 Apply，自动刷新 Source 与 target status。
66. 请求门禁期间 UI 显示 waiting/applying 并允许取消。
67. Ready with Issues 显示安全 target-specific issue，仍允许请求。
68. 日志、错误、审计与前端事件均不含 Tavily key 或 Authorization header。
69. 渲染翻译 UI 的前端测试显式初始化项目 i18n，且 stderr 无警告。
70. 最小相关 Rust、SDK、Frontend 与集成测试通过后，完整仓库 lint/test 通过。

### 手工端到端验收

- 使用本地开发构建和交互式提供的真实 Tavily API Key；不得提交、写入 fixture、出现在截图中，或作为本功能 CI secret。
- 同步官方 marketplace，通过 UI 安装已发布的 OpenCode Agent 与 Tavily MCP，确认 Tavily 初始为 Needs Configuration，且不影响既有对话。
- 保存 API Key，观察自动传播与就绪；确认托管文档只在仓库本地被忽略、权限受限，并包含预期 OpenCode HTTP entry。
- 只在 Ready 后开始新对话，让 OpenCode 执行 Tavily 支持的网页搜索，验证工具已暴露并返回真实结果。
- 新建 Workspace，验证首次允许 turn 前已自动获得 Tavily。
- Working turn 期间重置 API Key，验证空闲后才清理；随后卸载 Tavily，确认先移除托管配置再删除包。

## 不在范围内

- Secret Setting、SecretRef、OS credential vault 或安全环境变量间接引用；protocol v1 必须拒绝 Secret。
- 对 Tavily 当前 API Key 提供加密保护；当前包将其声明为普通 string，本期明确按明文处理。
- Workspace 级 MCP 开关、Agent 级用户选择或 Session 级选择。
- OpenCode stdio 物化；Core 与协议建模 stdio，但 OpenCode v1 只声明 HTTP。
- deprecated HTTP+SSE、MCP OAuth discovery、多 endpoint、fallback transport 或 Ora 启动 bundled HTTP server。
- 不经过 Agent 协调与 restart/resume 的热插拔。
- 普通配置重置或卸载时自动取消 Working turn；紧急凭据吊销属于独立功能。
- 将共享 OpenCode CLI 重构为每 Workspace 一个进程。
- 通过 ACP `session/new.mcp_servers` 传递配置。
- Host 渲染 OpenCode 或其他 Agent 原生 schema。
- 合并写入用户维护的 `.opencode/opencode.json`；该路径要么全量归 Ora 管理，要么视为冲突。
- 建立全局 plugin protocol major-version 框架。
- 为旧实验性 MCP 交付方式提供兼容层。
- 发布或修改 Tavily MCP，除非验证发现独立打包缺陷。

## 补充说明

- 以本 Workspace Effect 模型替换旧的启动前 MCP 配置规范。只有按顺序完成 Host、SDK、OpenCode、marketplace、自动化测试与 Tavily E2E，才能宣告端到端功能完成；分阶段发布时必须保留可追踪的发布门禁。
