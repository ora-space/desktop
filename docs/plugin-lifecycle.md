# 插件生命周期管理（Plugin Lifecycle）

> 本文档记录"插件生命周期管理"功能在实现前的设计共识。范围：扫描、激活、停止、卸载、启用、禁用六个动词 + 状态查询。本文是后端 only 的设计基准；前端改造为后续独立任务。

## 背景与现状

- `crates/plugin-manager` 目前只做**发现**（扫描 `<data-dir>/plugins/`*）。其 README 明确声明**不**负责安装、启用、禁用、移除或启动插件。它在 bootstrap 时调用一次 `PluginManager::discover`，作为不可变快照存放在 `DesktopState` / `AppState`。
- `crates/plugin-runtime` 能通过带帧 JSON-RPC stdio 协议 `launch`/`invoke`/（drop 时 shutdown）单个插件 Deno 进程，但目前**仓库内无任何调用方**。
- `crates/plugin-manifest` 解析的是*已发布 release* 的 TOML manifest（"从 registry 安装"范畴），同样在自身 crate 之外无人使用。
- 数据库里**没有插件表**（schema 已到 v0005）。前端的启用/禁用状态是内存 mock（`usePluginInstallStore`，Zustand store，关闭即重置），且前端**没有 activate/stop 的概念**，只有 install/enable/disable 的 mock。
- 分层约定：`domain → contracts → application（ports）→ backend（基础设施，如状态化` agent_runtime `actor）→ db`。
- `backend/agent_runtime`（最接近的先例：有状态 actor + 持久化）住在 `backend/`，同时使用 `ora_application::SessionRepository`（端口 trait）与 `ora_db::SqliteSessionRepository`（具体实现）。仓库对"有状态 actor"的既定模式是：actor 住 `backend/`，持久化走标准分层。
- 已有 `AppEventHub` + `AppEventPublisher` 广播中枢（`crates/backend/src/app_event.rs`），`agent_runtime` 用它往前端推事件，前端经 `watchAppEvents` 流订阅。现有 `AppEvent` 枚举：`Ready`、`SessionTitleUpdated { session_id }`（注释原话："Tells clients that the persisted session row should be queried again"）——即"带 id 的失效通知，前端收到后重新查询"的模式，与 VS Code `extensions.onDidChange` 同构。

## 一、范围边界

- **本任务 = 生命周期管理**：扫描、激活、停止、卸载、启用、禁用（六个动词）+ 状态查询。
- **install 不在范围**：插件包假定已在磁盘上（手动/未来安装功能放置）。`plugin-manifest`（release TOML 解析器）本任务**不使用**。前端市场目录 `PLUGIN_CATALOG` 继续是 mock。
- **invoke 不在范围**：前端 API 只交付六个动词 + 状态；调用插件方法留给未来的 agent-runtime 集成。`Backend` 内部持运行句柄（activate/stop 所需），但不暴露通用 invoke 命令。
- **前端不在范围**：只做后端。前端 mock 保持不动，靠命令/Rust 测试验证。

## 二、状态模型（两正交维度）

- **持久化维度**（跨重启存活）：`installed`（磁盘上有包，由文件系统派生）+ `enabled`（用户意图，存 DB）。
- **运行时维度**（仅内存，重启清零）：`activated`/`stopped`（进程跑没跑）。
- **不变式** `running ⇒ enabled`：运行中的插件必然 enabled；disabled 的绝不跑。

### 四动作语义

| 动作       | 行为                                                                 |
| ---------- | -------------------------------------------------------------------- |
| `enable`   | 置 `enabled=true`，**不**自动启动                                    |
| `disable`  | 置 `enabled=false`，**且自动 stop** 正在跑的进程                     |
| `activate` | 启动进程；**要求 enabled**，否则报错"先启用"；幂等（已在跑则 no-op） |
| `stop`     | 杀进程，**不动 enabled**；幂等；enabled/disabled 下都可调            |

设计依据：`enabled` 是"是否允许运行"的权威门槛（契合 AGENTS.md"让非法状态不可表示"）；`activate/stop` 是纯粹的运行时控制。用户把 enable 与 activate 分成两个动词，即"启用 ≠ 立即启动"，故 enable 不自动启动是一致的。

## 三、持久化与架构

- **enabled 存 SQLite 薄表** `plugin_state(plugin_id PK, enabled, 审计字段...)，不包括is_deleted`。插件**身份**仍由文件系统发现而来（DB 不存身份，避免 package_name/version 等派生数据同步问题）。DB 行**首次 enable 时才建**，保持最小。
- **生命周期编排在** `crates/plugin-lifecycle/`。持久化走标准分层：
  - `crates/domain/src/plugin.rs`：`PluginState` 实体。
  - `crates/application/src/plugin/ports.rs`：`PluginStateRepository` 端口。
  - `crates/db/src/repository/plugin.rs`：`SqlitePluginStateRepository` 实现。
- `Backend` 持有 actor、对外暴露六个生命周期方法。进程侧沿用 `plugin-runtime` 已要求的 `ProcessSpawner` trait 做注入测试。

设计依据：插件 `plugin-manager`/`plugin-runtime` 是无状态、无 domain 的独立"构建块"crate，类比 `ora-acp`/`ora-history`/`ora-scheduler` 之于 `agent_runtime`；编排它们的有状态 actor 的既定归宿是 `backend/`。

## 四、生命周期操作语义

### 开机行为

- **不自动激活**（贴合 VS Code 懒加载 / activation events 模型）。`enabled = 有资格`，激活靠显式 `activate`（或未来由 agent-runtime 等调用方按需触发）。
- 开机只做一次 scan + 对账。
- 设计依据：VS Code 并非开机即激活所有 enabled 扩展，而是按声明的 activation events 懒加载；`enabled/disabled` 是独立闸门（disabled 永不激活；enabled 只代表"有资格被激活"）。本任务不引入 activation events schema，保持范围收敛；未来若要"开机自启"可像 VS Code 的 `onStartupFinished` 加 per-plugin 激活策略声明。

### uninstall 流程

1. 若该插件正在运行 → **stop**（杀进程、丢弃 `PluginRuntime` 句柄）。必须在删目录**之前**——Windows 下进程持有 entrypoint 文件句柄时无法删除目录。
2. 删除 DB 里的 `plugin_state` 行（**删除**，非留 tombstone）。
3. 删除 `package_root` 目录。
4. 刷新内存中的 installed 快照（重新 discover）。

**not-found 条件**：目录没了**且** DB 无行才 not-found；目录没但 DB 有行（孤儿行）→ 清行（成功）。这让用户能显式清掉孤儿行，不会卡在 not-found。

### scan / refresh

重新 discover + 对账（见第五节）。

## 五、对账（reconciliation）

三处事实源——文件系统发现 / DB enabled 行 / 内存运行表——会在"插件目录被外部删除"等场景下不一致。对账把它们修一致。

- **时机**：只在手动 `scan_plugins` 和开机时。**不**加 FS watcher、**不**轮询（与 `plugin-manager` 既有"discovery 完成后不 watch"立场一致）。
- **规则**：

| 情况                                                 | 处理                                        |
| ---------------------------------------------------- | ------------------------------------------- |
| 已发现 + 无 DB 行                                    | `enabled=false`（默认；**不**自动建行）     |
| 已发现 + 有 DB 行                                    | 用 DB 行的 enabled 值                       |
| 运行表中存在、但已不被发现（目录被外部删）→ 悬空进程 | **stop 该进程**（安全：包都没了的进程应停） |
| DB 行存在但已不被发现（目录没了）→ 孤儿行            | **删除该 DB 行**                            |

- 不变量：**文件夹没了 ⇒ 状态也没了**。
- 删除孤儿行的理由：在"开机不自动激活"下孤儿行虽不触发启动，但会攒垃圾、让状态查询骗人；删它的代价极低（重新 enable 即可恢复），收益是 DB 长期干净、查询永远准确（契合 AGENTS.md"clean design / no legacy cruft"）。

## 六、失败 / 并发 / 状态

### 失败处理

- 启动失败或运行中崩溃 → runtime 变 `Failed(reason)`，`enabled` 不变（按不变式，失败不改变启用意图）。
- **不自动重试**，避免崩溃循环。用户/调用方再 `activate` 重试。

### 状态模型（对外）

每个插件暴露两个字段（对应两维度）：

- `enabled: bool`（持久化维度）
- `runtime: "stopped" | "starting" | "running" | "failed"` + 失败时的 `failureReason`（运行时维度）

### 并发

- 对**同一插件**的操作排队执行（按 plugin_id 串行），避免两个"激活"打架。
- 对**不同插件**的操作并行。
- 重复操作幂等：已在跑再 activate = no-op；已停再 stop = no-op。

### 卸载中途失败

- 不做事务回滚。stop + 清行成功、删目录失败 → 报错给用户；剩余目录下次刷新时作为"全新未启用插件"重新出现（自愈）；用户再点一次卸载即可删干净。

## 七、API 面（contracts + 命令/handler）

### 一个查询

`list_installed_plugins`：返回缓存快照（身份 + enabled + runtime + failureReason），**不重扫磁盘**。走 actor（拼接 发现×DB×运行表），不再直接读那个不可变快照。

> 对应 VS Code `vscode.extensions.all` 返回的 `Extension[]`——一个对象把身份（manifest）和状态（`isActive`）合在一起。

### 六个动作命令

各收 `{ pluginId }`，桌面 Tauri 命令，委托 `Backend`：

| 命令                               | 作用                                   |
| ---------------------------------- | -------------------------------------- |
| `scan_plugins`                     | 重新 discover + 对账，返回刷新后的列表 |
| `enable_plugin` / `disable_plugin` | 翻转 DB enabled（disable 顺带 stop）   |
| `activate_plugin` / `stop_plugin`  | 启/停进程                              |
| `uninstall_plugin`                 | 第四节的卸载流程                       |

每个动作返回**受影响插件的即时状态**（给前端即时反馈）。

### 事件流（用现有 `AppEventHub`）

- 新增 `AppEvent::PluginStatusChanged { plugin_id }`（给现有 `AppEvent` 枚举加一个变体，**不是新流**）。
- actor 在**每次状态流转**发布——含动作触发的，**也含异步的**（starting→running、running→failed 崩溃）。
- 前端（已订阅 `watchAppEvents`）收到后重新查 `list_installed_plugins`（缓存、便宜）。
- 设计依据：对应 VS Code `extensions.onDidChange`（动作不返回列表、发事件、消费方重查缓存数组），也对应 ora 已有的 `SessionTitleUpdated { session_id }`（带 id 的失效通知 + 重查）模式。
- 动作返回值给"动作做完那一刻"的即时状态；`AppEvent` 流给"之后异步变化"的后续状态——两者分工。activate 返回时还是 starting，等变 running/failed 时再发一次事件。

### 不全量重扫

动作只改内存状态 + 发事件；只有 `scan_plugins` 重扫文件系统。

## 八、实现落点（后端 only）

| 位置                                                                             | 内容                                                                                |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `crates/domain/src/plugin.rs`                                                    | `PluginState` 实体（plugin_id、enabled、审计字段）                                  |
| `crates/application/src/plugin/`                                                 | `PluginStateRepository` 端口 + README                                               |
| `crates/db/src/migration/schema_v0007.rs` + `crates/db/src/repository/plugin.rs` | 迁移 + `SqlitePluginStateRepository`                                                |
| `crates/contracts/src/plugin.rs`                                                 | 扩展 `InstalledPlugin`（加 enabled/runtime/failureReason）+ 6 个动作的请求/响应类型 |
| `crates/contracts/src/app_event.rs`                                              | 加 `AppEvent::PluginStatusChanged { plugin_id }` 变体                               |
| `crates/plugin-lifecycle/`                                                       | 状态机（运行表、discover、六个动作、对账）+ Deno runtime 适配器 + README            |
| `crates/backend/src/lib.rs`                                                      | `Backend` 持 actor、暴露方法                                                        |
| `apps/desktop/src-tauri/src/commands.rs`                                         | 6 个新 Tauri 命令 + 扩展后的 list，委托 `Backend`                                   |
| 前端                                                                             | 不动                                                                                |

## 附：关键不变式速查

- `running ⇒ enabled`（运行中必然 enabled；disabled 绝不跑）。
- 文件夹没了 ⇒ 状态也没了（对账删孤儿行）。
- `enable` 不改 runtime；`disable` 必连带 stop；`activate` 要求 enabled；`stop` 不动 enabled。
- 动作幂等：重复 activate/stop 为 no-op。
- 不开机自启；激活按需（显式 activate 或未来调用方触发）。
- 动作不全量重扫磁盘；只有 `scan_plugins` 扫描。
