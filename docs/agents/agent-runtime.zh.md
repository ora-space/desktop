# Agent 运行时

[English](agent-runtime.md) | 中文

`ora-agent-runtime`（`crates/agent-runtime`）拥有 Ora 能连到的所有 Agent 连接，并为每个 Ora 会话提供一个串行 actor。它用 ACP 与已安装插件提供的 Agent 通信，对自己运行在哪里一无所知：Desktop 通过 `ora-backend` 托管它，沙盒里的 Node 为 Cloud 运行托管同一份代码。记录、切换和历史见[会话历史](session-history.zh.md)；MCP 交付见 [Session MCP](session-mcp.zh.md)。crate 内部细节见 [`crates/agent-runtime/src/README.md`](../../crates/agent-runtime/src/README.md)。

## Agent 来自插件

有持久化命令队列的宿主可使用 `prompt_session_with_admission`。会话 Actor 先检查 provider 附着和 MCP
就绪状态，再调用宿主的短同步提交，随后才记录或发送 prompt。提交失败时拒绝该轮而不执行。
现有 prompt API 不带提交回调，保持原行为。

每个 Agent 都由一个已安装的 `agent` 类型插件提供；Ora 不内置任何 Agent，用户安装提供 Agent 的包之前一个都没有（每个包一个 Agent）。插件完整拥有它的 Agent 进程：决定运行哪个 CLI，并通过 `agent/acp` 通知通道把 ACP 帧转给宿主。见 [`plugin_agent/README.md`](../../crates/agent-runtime/src/plugin_agent/README.md) 和[插件系统](../plugins/plugin-system.zh.md)。

- **身份**：会话保存一个 `AgentRef`，即提供它的包的完整规范插件 id（`official/ora-space.claude`）。命名空间是身份的一部分，因为两个 marketplace 源可能发布相同的 identifier。没有 supervisor 认识的身份表示"当前未安装"，报告为 `agent_runtime_unavailable`。以更早、更短写法保存的值按原样携带，在用户重新选择 Agent 之前不匹配任何 Agent；没有任何东西会改写它们。
- **监管**：受监管的集合恰好等于已安装的 Agent 集合，每次安装、卸载和扫描都会重新协调，所以新装的 Agent 不需要重启。supervisor 通过插件生命周期附着到运行中的插件（`PluginApi::attach_agent`，必要时按需启动），通过无损 tap 读取该代的 `agent/acp` 通知，一代结束时请生命周期停止插件。
- **替换**：一个 supervisor 句柄只属于单次安装的实例：更新或重装会整个退役旧 supervisor，先前克隆的句柄不会自动跟随共享注册表里的替换。会话重新获取 provider 通道时，actor 按持久化的 `agent_ref` 解析当前 supervisor，并把它用于这次附着和 MCP 检查。因此既有对话无需重启 Ora 或停止会话，就能通过新 supervisor 继续；被卸载的插件继续报告 `agent_runtime_unavailable`。恢复和基于 transcript 的重建经由同一附着路径获取通道。
- **可执行文件**：操作系统进程归宿主所有：`ora/childprocess/spawn` 接受由操作系统解析的 `command`，或由宿主拼接到插件安装根目录上的 `packageCommand`。Agent 包要么附带 CLI（按目标发布），要么不附带（通用发布，在 `PATH` 中找用户自己装的）；插件先请求包内路径，宿主回答 `package_command_missing` 时再回退到 `PATH`。
- **失败分类**：机器上没有 CLI 报告 `agent_not_installed`，保持可重试，且不逐次记录日志。附带的可执行文件无法运行报告 `agent_unusable`，该 Agent 在本进程剩余时间内被放弃。

## 连接与监管

私有 `manager_startup` 模块负责管理器构造与持久会话状态修正，并在连接监督器启动前执行。

私有 `model_config` 模块为启动时的模型意图和 stale-load 重建识别同一个 provider 配置项：启动只应用明确提供的值，重建读取报告的当前值。空报告、两个 select 导致回退有歧义、或 `Model` 类别不是 select 时，报告不提供模型。其中 `model_selector` 规则是公开的，工作流执行器用它匹配图中的 `modelId`。

Backend 启动时，遗留的 `Running` 会话行被协调为 `Stopped`，然后每个 Agent 在专用的运行时线程中启动并执行 `initialize`。握手声明了会话配置选项能力（模型选择器依赖它）。每个 Agent 独立重试，指数退避从 250 毫秒到 30 秒封顶；即使所有 Agent 都失败 Ora 仍然可用，对不可用 Agent 的操作报告 `agent_runtime_unavailable`。一分钟内超过三次真实失败会打开该 Agent 的重启熔断：发布 `Failing`，本进程内不再重试。没有实现 Agent 契约的插件直接进入 `Failing`。

连接丢失时，该 Agent 进行中的操作失败，只把它注册的会话标为 `Stopped`，请生命周期停止旧进程，之后才启动替代进程。会话按需重新附着，prompt 从不自动重放。丢弃最后一个 Backend 所有者会停止所有 supervisor，取消路由中的操作，并结束每个进程树。

前端的 Agent 目录在运行时派生：选择器列出已安装的 `agent` 插件，使用各包自己的显示名和 logo，只提供状态为 `Ready` 或 `Starting` 的身份。`Unavailable` 会慢速轮询，因为它可能自行恢复；`Failing` 不会。

## 会话生命周期

打开或进入一个聊天不会创建任何 backend 状态。在 `startSession` 返回之前，前端把乐观的第一轮保存在本地。

- **启动**：`startSession` 是唯一的创建入口。它用当前的 Session MCP 快照调用 `session/new`，把调用方可选的模型意图应用到握手报告的选项上（应用成功后，Agent 此前推送的选项即过期），持久化会话，打开历史并安装 actor。在 actor 接手之前，一个 guard 会在任何错误或取消时释放未暴露的 provider session（支持时用 `session/delete`，否则用 `session/close`），持久化失败还会删除历史文件并软删除已插入的行。
- **加载**：load 只读取 Ora 自己的记录，不接触 Agent：不创建 provider session，不改变生命周期状态，所以即使 Agent 已经不在，对话也能打开。如果 actor 存在，由它自己回答 load（回放记录，再跟随进行中的 prompt），但不接管那个 prompt。处于附着状态时，回放之前先送出 provider 最近一次上报的选项和命令列表；两者都不写入记录，附着结束即失效。
- **附着**：prompt 才会获取 provider session。没有通道的会话先在当前连接上注册路由，用私有的 provider id 和当前 MCP 快照调用 `session/load`，丢弃 Agent 复述的内容，并把行标为 `Running`。握手得到的选项和命令在本轮之前以更新的形式送达客户端。Agent 通常在答复 setup 之后才上报命令，所以没有 prompt 进行时到达的命令列表会保留最新一份，在下一个 prompt 开头送达。空闲排空会丢弃其他更新，它们不会带入下一轮；标题轮询占用会话期间到达的更新则排队到下一个 prompt，其中命令列表同样只留最新一份，选项按送达时的值给出。受理会等待附着和 MCP 检查完成，所以无法连到 Agent 的发送会以"发送失败"的形式失败。
- **重建**：无法 load 或恢复失败的 Agent，会得到一个用同一快照 `session/new` 出来的替代 provider session，下一个 prompt 携带已记录的 transcript，与切换 Agent 完全一样。Agent 也可能对已不存在的会话误报 load 成功：该通道的首个 prompt 如果没有任何 assistant 输出或工具调用就失败，runtime 会关闭它、重建、推送新的 setup 更新，并携带 transcript 与恢复会话报告的 model 重发同一 prompt 一次；重试轮仍失败则返回该轮错误，只有重建失败才返回第一次错误。该兜底只对可复用的 `RequestFailed`、且尚未产生任何输出的 prompt 生效；首个 prompt 一旦被取消或正常结束，就不再保留这次重试资格。第一次 attempt 期间注册的 load follower 会跨重建跟随同一逻辑回合，只收到一次终态边界，并收到替代会话的选项和命令。旧会话上报过的命令会被撤下，直到替代会话上报自己的；替代会话不提供任何选项时，旧选项也会被撤下。
- **配置**：持有 provider 的会话报告自己的选项，通过 `setSessionConfig` 配置。请求由 actor 发给 Agent，actor 是会话选项的唯一持有者：答复和 Agent 自己的 `config_option_update` 按从连接读到帧的先后采用，不论由回合、标题轮询、MCP 刷新、会话 setup 还是空闲时读到。答复先采用再返回，所以之后打开的视图看到的是改动后的选项。调用方最多等 30 秒；provider session 还在时，之后到的答复仍会采用，并由进行中的回合送给视图。Agent 不读取时，等待的是调用方，不是 actor。多个在途的设置互不等待。provider session 在答复之前没了，在途的设置当场结束：由宿主放弃的算成功（`Stop`，切换和删除到 actor 也是它；回合的接收方不再读取；重建，调用方得到替代会话的选项），失去 Agent 的算失败。还没写给 Agent 的设置遇到 `Stop` 也失败，因为没有向 Agent 请求过它。没有 provider 的会话没有可改的选项：设置返回失败，这期间选的模型是一个意图，在下次附着时应用。推理强度只从活跃会话报告的 `thought_level` 选项读取，没有会话之前的强度目录。
- **选项送达**：视图持有 actor 选项的副本。有 provider 的会话，每条加载流、跟随流和 prompt 流都以当前选项开头（空列表也带），之后随流送出 actor 采用的每个选项帧。每采用一帧（不论值是否变化、是否在回合中），以及会话得到或失去 provider，都会发应用事件 `session_options_changed`，只带会话 id，不带选项。`getSessionOptions` 不访问 Agent，读出当前选项，会话没有 provider 时为 `null`；回合、标题轮询、MCP 刷新期间立即答复，附着等不读命令的步骤完成后答复。没有流的视图每收到一次通知就读一次，同时只读一次，读的期间又收到通知就读完再读；读的期间流开始了，这次结果作废。有流的视图收到通知只记下，流结束时如果记下过通知，或流没有送来选项，就读一次。通知可能丢失，所以视图每次订阅应用事件后都会重读屏幕上的会话。视图得知会话没有 provider 后保留最后的选项，模型选择器改为记意图，推理强度控件隐藏，只读选择器继续显示最后的模型。Node 没有选择器，忽略这个通知。
- **停止与删除**：显式停止在 Agent 声明支持时调用 `session/close`，卸载路由并保留 provider 历史。删除见[会话历史](session-history.zh.md#删除)。

前端按 Agent 记住（只存在浏览器存储中）最近一次在未开始的聊天中选的模型，只在该 Agent 当前目录仍然列出它时才提供。

### 发现

会话之前的模型和命令来自插件的 `agent/discover`（旧插件是 `agent/list_models`，不知道命令），按需调用，带 Workspace 的工作目录，预算 60 秒。Ora 不缓存结果；发现和缓存都归插件，如果插件需要 ACP，它通过宿主的子进程 API 启动一个单独的一次性进程，而不是借用 Ora 的共享连接。失败只影响这一次请求；空列表是合法的。

调用方说明是否需要命令。需要时，插件等待 Agent 的第一条命令上报，最多等到 `session/new` 响应之后两秒，也不占用清理预留的时间；没有等到就不带命令返回。只需要模型的调用方（工作流编辑器、工作流节点的模型选择器、定时任务校验）不要求命令，所以从不等待。它们在前端有自己的缓存条目：不带命令的结果不能挡住对话输入框的命令。

命令可能来自 Skill，所以需要命令的发现带上此刻建立的会话会得到的 Skill 可读根和目录，以及一个修订号。每次提交 Skill Catalog 变更（安装、卸载、改名、升级、原地编辑），原地编辑、改名或删除已经在磁盘上换过或移走包、却因为写不进数据库而回滚时（推进一次），或发现可用的 Skill 集合变了（无论是读会话的 Skill 目录还是设置页列出 Skill 时），Desktop 宿主都推进修订号，并同时为每个 Agent 发布 `AgentModelsInvalidated`。修订号只增不减，只在内存里。失效不重启 Agent，没有界面在展示就不重新发现，只取模型的结果不受影响。

输入框展示会话上报的命令；会话还没有上报、或有待执行的 Agent 切换时，展示下一条消息目标 Agent 的发现结果。输入的命令是否不受支持只由会话自己的上报决定，因为发现不带 MCP 服务器。命令是不可信文本：输入框按纯文本展示，最多 500 条，丢弃为空、超过 128 字节或含空白、控制、格式字符的名称，描述截到 4096 字节，提示截到 256 字节。会话交付的内容不截断；会话上报超过 500 条时，Desktop 日志记一条警告。

### 权限

Ora 目前没有用户可配置的审批策略。活跃的 actor 立即答复每个权限请求，走的是与用户选择相同的路径，优先选择 `AllowOnce`，只有 Agent 没有提供一次性选项时才用 `AllowAlways`。记住的选择由 Agent 自己持久化：Claude 会把它写进项目的 `.claude/settings.local.json`，在 Node 上这就是用户仓库的 checkout，会随 Revision 交付；在 Desktop 上则会留下一条用户从未做出的批准。不提供任何允许选项的请求会取消本轮。没有 prompt 进行时到达的请求以取消答复。

### 首个标题

新附着的会话有一个只存在于运行时的标题窗口。合法的 `session_info_update` 标题立即接受。第一个 prompt 以 `EndTurn`、`MaxTokens` 或 `MaxTurnRequests` 结束后，声明了 `session/list` 的 Agent 会在第三秒和第十秒被询问（只在 actor 空闲时发请求，超时五秒）；否则窗口在第十秒关闭。最后一个成功持久化的合法标题胜出，重复的标题不写入。用户改名会锁定窗口。恢复的会话不会重新打开窗口。只改标题的写入之后，actor 发布 `SessionTitleUpdated`，提示客户端重新拉取。

## 流控

ACP 的 stdout 是按行分隔的 JSON-RPC。连接读取器把帧无界、有序地交给中央路由器；每个已注册的会话拥有一个 256 项的有界 FIFO，存放更新、权限请求和终止响应。连接丢失和队列溢出走单独的控制通道，所以终止性故障不会被满的 FIFO 挡住。一个会话溢出只停止这个会话。

- 在 `session/new` 返回 provider id 之前，通知暂存在一个最多 256 条的共享缓冲里；注册时取出匹配的通知，最后一个并发 setup 结束时丢弃其余的。`session/new` 的答复不经过这个缓冲，所以每一帧都带着读到它的先后：答复带选项时，早于它读到的选项更新，无论还在缓冲里还是注册后才路由到，都会丢弃，因为答复已经取代了它。不带选项的答复不取代任何选项，这些更新照常保留。
- 路由绑定到连接的代；来自旧连接或已卸载会话的更新作为过期内容丢弃。
- Agent 发起的未知请求得到 `-32601` 响应，连接不会因此结束；格式错误的帧、无法匹配的响应、超大帧和 stdio 丢失都是连接故障。
- load 或 prompt 只有在消费了与之匹配的响应之后才完成，因此之前的每条更新都属于这次操作。
- 历史回放是唯一施加背压而不是快速失败的流。

丢弃前端迭代器或关闭 Tauri 流会发送 `session/cancel`。取消作用于整个会话：即使从一个只是跟随的视图发出，也会到达 prompt 的所有者，记录一个已取消的回合，actor 仍可复用。

## 上限

| 项目 | 取值 |
|---|---|
| `initialize` | 15 秒 |
| 插件发现 | 60 秒 |
| `session/new` / `session/load` 无活动 | 30 秒；交付 MCP server 时为 120 秒；setup 通知会重置 |
| prompt 无活动窗口 | 45 秒，之后最多三次重试依次为 60、90、120 秒；工具运行和等待权限时暂停；未结束的 CodeAgent 委托由其子代理帧续期 |
| 取消结算宽限期 | 5 秒 |
| 连接重试退避 | 250 毫秒，翻倍至 30 秒 |
| 重启熔断 | 1 分钟内超过 3 次失败时打开 |
| 会话队列深度 | 256 项 |
| 结构化 prompt 大小 | 序列化后的 JSON 16 MiB |
| 交接 transcript 大小 | 不限 |

### setup 无活动

setup 按无活动计时，而不是总时长。交付 MCP server 的请求有 120 秒，因为 Agent 要先连上这些 server 再回答，而真实的 MCP 初始化可能要几十秒；不带 server 的请求仍是 30 秒。创建过程中等待 id 时发出的通知会重置期限，所以持续报告进度的 Agent 不会被截断，沉默的 Agent 则以 `agent_timed_out` 失败。

### prompt 无活动与重试

每个 prompt 有一个 `PromptInactivityPolicy`。`Timeout`（普通聊天和默认值）应用无活动窗口；`Wait` 每轮只发送一次，达到工作流静默上限（固定 30 分钟）时本轮失败。有限等待在工具运行和权限应答期间持续计时，只有可归属的有效进展才能续期。长时间静默的节点可显式选择 `Unbounded`。见[工作流运行](../workflows/workflow-runs.zh.md#prompt-无活动)。

在 `Timeout` 策略下，Agent 消息、思考、计划和工具生命周期更新会重置窗口；会话外围的通知（`available_commands_update`、`current_mode_update`、`config_option_update`、`session_info_update`、`usage_update`）不会。运行中的工具会暂停窗口直到最后一个并行工具结束，等待权限也会暂停到得到答复，所以一个长工具可以运行数小时。prompt 没有绝对时长上限。

CodeAgent `_subagent/update` 帧是唯一的投影例外：只要其外层 `toolCallId` 指向本次尝试中仍为 `pending` 或 `in_progress` 的父委托，每一帧都为父窗口续期一个完整窗口但不暂停计时，重试则清除旧尝试的父委托观察重新开始（续期依赖父 `toolCallId` 在同一 provider 会话内唯一；若 provider 在新委托上复用同一 id，旧任务迟到的子帧仍可能为其续期）。实测 CodeAgent 父 `Task` 从不报告 `in_progress`，这一续期仅在子代理持续按窗口发帧期间保持前台委托存活。续期只看子帧是否到达、不判断子代理是否健康，在 CodeAgent 为父调用补发 `in_progress` 前有两个接受的边界：一是子代理内单个正常工具运行超过当次窗口且期间没有中间帧（或子代理停止发帧）时，父 prompt 仍会被取消并重试，使子代理和该工具重跑；二是子代理即使陷入错误或重试循环，只要持续发帧就会无限续期父窗口，须手动停止。

窗口到期时，运行时发送 `session/cancel`，并在五秒宽限内等待卡住的那个请求的响应。如果响应带着 `stop_reason: cancelled` 到达且还有剩余窗口，就在同一个 provider session 上重发同一条用户 prompt，之前先在所属的流上发出 `retrying { retry, maxRetries }` 标记。必须有这个响应作为分界，因为 ACP 更新不带请求 id。缺少分界、其他 stop reason 或次数用尽时，prompt 以 `agent_timed_out` 失败，只隔离这个会话。重发复用会话，一条用户消息只记录一次。运行时在取消排空后写入 `promptRetried` 边界，排空期间的更新归入被放弃的尝试。宿主为新发送刷新尝试指令和完成 token。

`Wait` 仍会在正常响应、provider 错误、连接丢失、溢出、显式取消、所有者清理和关闭时结束，setup 和配置期限也仍然有效。它无法区分健康的隐藏子 Agent 和卡住的 provider，所以用户可能需要手动停止。

## 错误

每个失败都以在产生处选定的类型化公开错误到达客户端。进程无法启动报告 `agent_start_failed`；所有运行时期限报告 `agent_timed_out`；列出模型失败报告 `agent_model_discovery_failed`。这些归为 `Unprocessable`，因为 CLI 缺失、不可用或太慢是用户可以处理的本地情况。队列溢出和 ACP 违规仍是 `internal_error`，因为它们是 Ora 的缺陷，其 request id 必须可以上报。插件编写的启动文本从不进入公开错误参数，只进入日志。

prompt 是有序的 ACP `ContentBlock`（文本、图片、音频、资源链接、内嵌资源）。空列表或只有空白文本会被拒绝。

## 宿主

宿主可选择启用 `RuntimeEvents::record_preview`，在定型前观察已合并的待完成记录。通知最多每秒十次，轮次结束发送空的
完整替换。回调为尽力而为，不产生确认，也不改变 `record_settled` 原有持久顺序。未选择启用的宿主不复制待完成快照。

宿主通过 `AgentRuntimeHost` 组装运行时，它的关联类型提供六个接口；它们是泛型参数，所以每个宿主得到自己单态化的运行时。

| 接口                 | 运行时需要                                    | Desktop                       | Node                         |
| -------------------- | --------------------------------------------- | ----------------------------- | ---------------------------- |
| `SessionStore`       | 创建、读取、列出和更新会话行；软删除          | `SqliteSessionRepository`     | `MemorySessionStore`         |
| `AgentAttach`        | 列出 Agent 插件，附着到进程和 tap，停止进程   | `PluginApi`                   | 本次执行自己的单插件生命周期 |
| `SessionSetup`       | Session MCP 视图、revision 和健康观测         | Session MCP 解析器和 MCP 健康 | `NoSessionMcp`               |
| `Skills`             | 当前 Available Skill 目录条目与可读存储根目录 | `DesktopSkillCatalog`         | `NoSkillCatalog`             |
| `RuntimeEvents`      | 标题和模型目录通知；写入的每一行历史          | `AppEventPublisher`           | 每一行成为一个 Thread 事件   |
| `WorkspaceDirectory` | Workspace 的 `cwd`                            | Workspace 仓储加路径基准      | 本次执行的 checkout          |

在 Desktop 上，`ora-agent-runtime` 会在活跃会话的首条 prompt（以及冷启动 `session/load`、重新打开已停止会话或压缩重置后的下一条 prompt）中注入全量 `<available_skills>`，在会话中途 Skill 集合变化时注入按路径计算的 `<available_skills_update>` 增量（若有前导 `/命令` 文本块，则紧随其后），并在 Agent 支持 `additionalDirectories` 时于 `session/new` / `session/load` 中传递两个稳定的可读存储根目录及 `_meta["ora/skillDirectories"]`。`NoSkillCatalog` 使 Node 会话保持不变。

失败以 `RuntimeError`（分类、类型化公开错误、上下文、错误链）离开 crate；Desktop 把它无损地转换为 `BackendError` 并能转回。crate 不得依赖 `ora-backend`、SQLite 或 Tauri。没有人在场的宿主使用的入口（`wait_for_agent`、`start_session_with_id`、`prompt_session_as_message`）见 [Node Agent 会话](../node/agent-sessions.zh.md)。

## 运行观察

Workspace 和工作流对话提供只读运行详情：最近实时收到的内容、工具或计划活动及客户端接收时间、
已知待处理权限请求、当前工具，以及已识别的 provider 重试/上下文压缩报告。缺少报告显示未知。
历史重放不刷新实时活动时间；切换 provider 或开始新 prompt 会清除旧观察。静默不能证明健康或卡死，
父回合状态不代表子代理生命周期。父活动与子活动的时间和工具分别显示，已知子工具在省略 metadata
的后续增量中仍归为子工具；历史或尚未观察到来源的工具显示来源未确认。观察时长不是超时倒计时。
界面只列出宿主实际暴露的请求；当前 runtime 自动处理权限，未支持的问题表单不会伪造为待处理请求。

OpenCode 子会话投影与 `_subagent/update` 扩展投影仍会显示，且从不暂停父 prompt 的无活动窗口。带有效外层 `toolCallId` 的 `_subagent/update` 帧，若其指向的父委托在本次尝试中仍为 `pending` 或 `in_progress`，还会为父窗口续期一个完整窗口：实测 CodeAgent 父 `Task` 在整个委托期间保持 `pending`，若不续期，超过首个窗口的前台委托会在子代理持续按窗口发帧期间被取消并重发。父委托已完成、失败、外层 id 不认识或缺失时不续期；重试会清除上一次尝试的父委托观察（假设父 `toolCallId` 在同一 provider 会话内唯一）。子工具后续省略 metadata 的增量仍保持子来源，但不续期。父委托工具自身的 in_progress 继续暂停既有计时，暂停优先于续期；显式 Wait 和取消/错误收尾不变。
