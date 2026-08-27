# Ora 插件体系与 rtk hook 插件打包调研

> 调研日期: 2026-08-26 | 仓库: E:/claude_code_project/desktop-hook | 调研范围: ora 插件管理/设计/市场/规约 + rtk 工程

## 1. 摘要

本次调研横跨 ora 插件系统的源码架构、设计规约（specs）、市场与 SDK、项目工程规约，以及 rtk 工程本身五个维度，目的是在动手设计/打包之前澄清“rtk 能否作为一个 ora hook 插件”这一核心问题。

结论明确：**ora 当前不存在任何“hook”插件类型（plugin kind）或“hook”能力（capability）。** 源码层面 `PluginKind` 是一个封闭的 5 变体枚举 `Workbench / Agent / Webview / Skill / Mcp`（来源：crates/plugin-manifest/src/enums.rs:47-93），specs 层面对 `hook` 关键字全文 grep 返回零匹配，且规约明确把插件类型限定为 `agent / mcp / workbench / webview` 四种（来源：specs/active/plugin/1-capability.md:118-143、5-mcp.md:40、6-workbench.md:60、7-webview.md:65）。rtk 作为一个“命令执行前改写、执行后过滤输出”的代理，既不提供 agent，也不是纯配置，无法直接落入任何现成类型。

因此把 rtk 打包为 ora hook 插件，本质上需要新增一个 `hook` 类型/能力，端到端贯穿 manifest、manager、contracts、lifecycle、registration 各层，并补上对应的设计规约。本文第 7 节给出三条可选路径及差距分析，第 8 节给出设计建议与待用户拍板的问题。

## 2. Ora 插件系统架构（crates 层级与数据流）

ora 的插件系统在 crates 层级上是一个分层管道，从 manifest 解析到运行时托管逐层加宽职责：

```
ora-plugin-manifest   解析/校验 orax.toml（只读域对象 PluginManifest）
        │
   ┌────┴─────┐
ora-plugin-registry  ora-plugin-manager
(marketplace git       (发现 installed 包、宿主侧校验、
 同步、构建             install/import、快照 InstalledPlugin)
 registry_index.json)
        │
   ora-plugin-config  编译 assets/config.json（MCP 配置、Settings）
        │
   ora-plugin-runtime  Deno 子进程 + stdio JSON-RPC 帧协议
        │
   ora-plugin-lifecycle 插件进程的唯一所有者；串联发现 + 持久态
        │                + 运行时 + 通知数据面 + surface-closer
   ┌────┴──────┐
ora-application    ora-backend
(定义 PluginState    (PluginApi/PluginGateway 具体装配：
 Repository 端口)     SQLite 仓库、DenoPluginRuntimeLauncher、
                      BroadcastNotificationSink；agent_runtime
                      挂载 agent 插件)
```

### 关键类型与 trait

- `PluginKind` 是封闭 5 变体枚举 `Workbench / Agent / Webview / Skill / Mcp`，`as_str()` 映射到 `workbench/agent/webview/skill/mcp`，`FromStr` 对任何其它值返回 `PluginKindError::Unsupported`（来源：crates/plugin-manifest/src/enums.rs:47-93）。
- `PluginManifest` 是从 `orax.toml` 解析出的不可变域对象，仅能通过 `parse()`（release 形态）或 `parse_installed()`（installed 形态）构造；字段私有、只读访问器；`validate_kind_sections`（manifest.rs:269-313）限定 `[workbench]` 只能与 Workbench 搭配（可选），`[webview]` 必须与 Webview 搭配（必选），Agent/Skill/Mcp 拒绝这两种节（来源：crates/plugin-manifest/src/manifest.rs:15-236）。
- `PluginContribution` 是 manager 侧的 kind↔payload 配对枚举：`Agent(InstalledPluginAgent) / Workbench(InstalledWorkbenchDescriptor) / Webview(InstalledWebviewDescriptor) / Skill(InstalledSkillDescriptor) / Mcp(InstalledMcpDescriptor)`；`entrypoint()` 仅对 Agent/Workbench 返回 `Some`，Webview/Skill/Mcp 无进程入口（来源：crates/plugin-manager/src/validation.rs:23-51）。
- 前端线协议 `InstalledPluginContribution`（ts_rs 导出）是独立的 tagged 枚举，`serde tag="kind"`、`rename_all="snake_case"`，变体 `agent/workbench/webview/skill/mcp`，只暴露展示字段，不含 asset 路径（来源：crates/contracts/src/plugin.rs:19-36）。
- `PluginManager::discover(data_dir)` 在 `<data-dir>/plugins/installed/<ns>/<name>/<version>` 下发现包，选最高 SemVer，解析 `orax.toml`，施加宿主侧策略（`main.js` 存在性、workbench `assets/index.html`、webview origin/download 规则、skill `SKILL.md` 树、MCP `assets/config.json`），返回不可变 `InstalledPlugin` 快照与发现问题；它**不**负责 enable/disable/启动进程（来源：crates/plugin-manager/src/lib.rs:40-70）。
- `PluginRuntime::launch` 通过注入的 `ProcessSpawner` 派生 Deno 子进程，等待 `ora/register` 握手，暴露 `invoke()`/`notify()` 与一个无界 `mpsc::UnboundedReceiver<PluginNotification>`；帧格式 `[4-byte BE len][1-byte type][JSON]`，`invoke()` 受 `call_timeout` 约束，`notify()` 不受约束；注册（methods/emits/effectSurfaces）在握手后不可变（来源：crates/plugin-runtime/src/lib.rs:96-355）。
- `PluginLifecycle<Repository, Clock, RuntimeLauncher, StatusPublisher, Sink>` 是插件进程的**唯一所有者**，串联发现、持久化 eligibility（`PluginStateRepository` 端口）、进程作用域运行时态（`PluginRuntimeLauncher`）、应用失效（`PluginStatusPublisher`）、通知数据面（`PluginNotificationSink`）。`enable_plugin` 对 Agent kind 还会启动进程（supervisor 挂接到运行进程）；Workbench/Webview/Skill/Mcp 仅记录 eligibility（Stopped）（来源：crates/plugin-lifecycle/src/lib.rs:101-679、launch.rs:32-154）。
- 生产适配器 `DenoPluginRuntimeLauncher` 通过 `ora_process::TokioProcessSpawner` 启动 Deno，并把 JSON-RPC `PluginRuntime` 包成 `DenoPluginRuntime`，暴露 `.process()` 让 agent supervisor 直接对话进程协议；默认超时 ready=10s、call=30s、shutdown=5s（来源：crates/plugin-lifecycle/src/runtime.rs:33-90）。
- 进程树终止用平台原语：Unix 进程组 `kill(-pgid)`，Windows Job Object 带 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `TerminateJobObject`（即近期 commit 所说的“Job Object FFI”），`ProcessTree` 持有 windows HANDLE，`unsafe impl Send/Sync`（来源：crates/process/src/tree.rs:1-189）。
- 所谓“runtime actor”实为 ACP agent runtime 的 per-session 串行 actor（crates/backend/src/agent_runtime/actor.rs），**不是**插件 runtime；`PluginRuntime` 自身是一组 tokio task（writer/reader/stderr/supervisor）。`AgentRuntimeManager` 每个 agent 持有一个受监督的 ACP 连接，通过 `PluginApi::attach_agent` 拿到 `PluginGenerationLease` + 无损通知 tap；`plugin_agent` 模块校验 agent 契约（`agent/start|stop|listModels` + emits `agent/acp`）并转发 ACP（来源：crates/backend/src/agent_runtime/README.md、plugin_agent/README.md、connection.rs:139-894）。
- 应用层装配：`crates/application/src/plugin/mod.rs` 只定义 `PluginStateRepository` 端口；具体组合在 `crates/backend/src/plugin.rs`：`BackendPluginLifecycle = PluginLifecycle<SqlitePluginStateRepository, SystemClock, DenoPluginRuntimeLauncher, AppEventPublisher, BroadcastNotificationSink>`；`PluginApi::open`（plugin.rs:181-228）构造 lifecycle + SQLite 仓库 + 时钟 + `DenoPluginRuntimeLauncher` + `AppEventPublisher` + `BroadcastNotificationSink`，暴露 list/scan/enable/disable/activate/stop/uninstall/install/import + attach_agent + replace_agent_effect_surfaces；`bootstrap.rs`（98-254）打开 `PluginApi`、构建 `AgentRuntimeManager`、暴露 `plugin_gateway()` 返回 `PluginGateway`（来源：crates/backend/src/plugin.rs:42-228、bootstrap.rs:98-254）。
- surface/desktop 层通过 `PluginGateway`（plugin_gateway.rs:36-126）访问插件，它包装 `PluginApi` 并向 lifecycle 安装 `SurfaceCloser`（`set_surface_closer`），使 stop/disable/uninstall 在停止进程前先关闭 desktop surface；surface crate 不直接 import plugin-runtime。
- `validate_registration` 是 kind 特异的握手契约闸门：Agent 无条件接受（契约由 agent supervisor 稍后校验）；Workbench 可注册良好方法但不能有 emits/effectSurfaces；Webview/Skill/Mcp 不能注册（无进程）（来源：crates/plugin-lifecycle/src/registration.rs:20-58）。
- agent 插件的 ACP 流量走 `agent/acp` 通知（非 invoke），因为 ACP 帧自带 id/相关性且可能持续数分钟，再套一层 `PluginRuntime` id + `call_timeout` 会截断长 prompt；宿主是纯管道，从不解析 ACP payload（来源：plugin-agent-runtime.md §5 lines 205-230、§7 lines 274-294）。

### 运行时托管模型

插件进程托管 = 子进程（Deno），经 `ora-process` 的 `TokioProcessSpawner/ManagedProcess` 启动，通过 framed stdio JSON-RPC 通信；Windows 上进程树用 Job Object 终结。只有 Agent 和 Workbench kind 会 spawn Deno 进程；Webview/Skill/Mcp 无进程。

## 3. Ora 插件设计规约（specs 能力分类法）

### 插件模型

ora 的插件是一个 Deno 零权限进程运行 `main.js` 作为胶水代码；宿主是“内核态”，SDK 是系统调用边界，插件进程默认零权限，任何 FS/network/subprocess/env 权限都必须经 Ora SDK 请求，`Deno --allow-*` 一律不生效（来源：specs/active/plugin/0-overview.md:5-47）。

### Manifest schema

插件 manifest 是 `orax.toml`（TOML），含 `resolver`、`name`/`identifier`、`namespace`、`kind`、`version`、`description`、`homepage`、`license`，以及 kind 特异表：`[permissions]`、`[workbench]`、`[webview]`、`[[targets]]`、`[head]`、`[dependencies]`（来源：specs/active/plugin/1-capability.md:118-143）。

### 能力/kind 分类法（关键）

specs 定义的能力/权限模型**不按 kind 索引**，而是共享权限分类法，在 `orax.toml` 的 `[permissions]` 下声明：`plugin_data`（access none/read/read-write）、`plugin_logs`（write bool）、`process`（executables/stdio/signals）、`process.sandbox`（workspace/network/environment）。规约明确：“后续增加文件选择器、HTTP 请求、系统通知等宿主能力时，应在 permissions 下增加各自的强类型配置”——即新宿主能力以新的强类型 `[permissions.*]` 条目形式加入，而**不是**以新的 plugin kind 加入（来源：specs/active/plugin/1-capability.md:110-239）。

### 完整 kind 分类法与 hook 是否存在

specs/active/plugin/ 下定义的 plugin kind 是**封闭的四种**：`agent`（1-capability.md:122）、`mcp`（5-mcp.md:40）、`workbench`（3-registry.md:14、6-workbench.md:60）、`webview`（7-webview.md:65、92）。注意：源码 `PluginKind` 有 5 个变体（多一个 `Skill`），但 specs 文本中 skill 作为独立 kind 的规格化描述在 active/plugin/ 下未单独成文（Skill 在 effect 规约和代码中出现）。**对 `hook` 关键字在整个 specs/ 树 grep 返回零匹配。** 因此规约层面也不存在 hook 能力。

各 kind 差异：

- `agent`（最重要）：运行 `main.js`，管理 AgentInstance/session，把一个 Agent CLI/Runtime 适配到 ora 统一 Agent 协议，实现 `ora/agent/start_agent`、`stop_agent`、`list_models`、`configure_agent`；`main.js` 不能直接创建 Agent 进程，必须用 Ora SDK（来源：specs/active/plugin/4-agent.md:1-113）。
- `mcp`：纯配置型，无 `main.js`、无 Deno 进程、无 SDK 调用；在 `assets/config.json` 声明 transport（stdio 或 http），由目标 Agent CLI 启动/管理 MCP Server；`McpTransport` 是互斥 tagged union（`Stdio{command,args,env}` | `Http{url,headers}`）让非法状态不可表达（来源：specs/active/plugin/5-mcp.md:1-49、171-245）。
- `workbench`：运行 `main.js` 且提供 Tauri 内 HTML/CSS/JS webview，页面通过受限 bridge（`plugin_webview_invoke`，受 Tauri ACL 与 `effective_methods` 交集约束）回调同一插件 `main.js`；methods = manifest `[workbench.methods]` ∩ runtime 注册 methods（来源：specs/active/plugin/6-workbench.md:1-84）。
- `webview`：纯配置（无 `main.js`），打开外部 HTTPS 站点于原生 WebView，维护 per-plugin browser profile，强制 allowed_origins，按 URL 快照规则接管 download（`DownloadDisposition` 互斥：Auto/Prompt/Reject）（来源：specs/active/plugin/7-webview.md:1-103、186-196）。

### SDK 变更（0a-sdk）

`specs/changes/plugin/0a-sdk.md` 实际是单行占位（“插件代码查询插件版本号”，37 字节），无 schema、无 API、无验收标准，是一个待补的 change note，并未给出实质 SDK 设计（来源：specs/changes/plugin/0a-sdk.md:1）。其余 change spec：`1-capability.md`（change）是 plugin runtime **日志**能力的实现指引（`permissions.plugin_logs` + `logs/` 目录）；`7-webview.md`（change）是 webview kind 实现指引 + 从 Skill Marketplace 抽取共享安全下载落地能力（`ora-utils::download_target`）。

### 规约工作流

`specs/` 分三层：`active/`（权威当前规格，按 feature 区编号文件）、`changes/`（实现指引/变更提案，编号匹配所修改的 active 节，字母后缀如 `0a-sdk.md` 表示新增子节）、`drafts/`（当前为空，新能力提案入口）。`docs/spec-management.md` 只描述 `specs/` 作为**只读评审面**的发现与展示，不定义 draft→active 的正式晋升流程；draft→change→active 是由目录命名与 change spec 引用 active 节号的惯例推断出的 de-facto 流程（来源：docs/spec-management.md:14-66）。

### 与 hook 的相关度

**明确结论：ora 无 hook 插件 kind，也无 hook 能力。** 最接近的现成 kind 是 `agent`（运行 `main.js`、宿主经稳定 `ora/agent/*` 方法调用、管理 start/stop 生命周期——与 hook 插件所需形态相同）和 `workbench`（已定义 host→main.js invoke 模型，由 manifest 方法表 ∩ runtime 注册方法交集约束——正是一个 hook 拦截点 allowlist 会用的模式）。要新增 hook 能力，至少需在 spec 中定义：hook 点封闭枚举、命令/消息拦截模型（观察者 vs 拦截器）、输出过滤管线（排序、幂等、大小限制、错误分类法）、注册与 generation 模型（`[hook.events]` ∩ `ora/register.params.events`，复用 `PluginGenerationLease`）、安全边界（零权限 Deno、宿主持有身份、新增 `[permissions.hook]` 强类型配置）、生命周期/install 不变量、与 Effect reconcile 系统的交互（不绕过 `AgentTarget` 的 WaitingForIdle/Quiescing），并补一份 `specs/changes/plugin/8-hook.md` 实现指引（来源：specs/active/plugin/{0-overview,1-capability,6-workbench,7-webview}.md、effect/2-declaration.md:148-168）。

## 4. Ora 插件市场与 SDK

### 市场源配置

市场源持久化在 `<data-dir>/plugins/marketplace_sources.json`，shape 为 `{"sources":[{"url":String,"branch":String}]}`，原子写、`deny_unknown_fields`，首次打开时种入默认源 `https://github.com/ora-space/marketplace`（branch `main`）。`add()` 拒绝重复 URL，`delete()` 按 URL 删除；源经 `RegistrySource::try_from_git` 校验（拒绝非 HTTPS URL 与畸形分支名）后持久化（来源：crates/backend/src/marketplace_sources.rs:11-131）。线协议 `MarketplaceSource{url,branch}` 经 ts_rs 导出，请求/响应对均 camelCase（来源：crates/contracts/src/plugin.rs:247-302）。

### 市场仓库结构与 index

缓存 registry index 位于 `<data-dir>/plugins/cache/registry_index.json`，schema `{updated_at:i64, version:"1.0", plugins:Vec<RegistryEntry>}`，经 `ora_utils::atomic::write` 原子写；`build_all` 扫描多源目录，按 namespace/name 去重（源序优先 first-wins），按 id 排序；缺失文件在 API 层返回空目录而非错误（来源：crates/plugin-registry/src/index.rs:13-129）。

`ora-space/marketplace` 仓库布局为 `registry/<首字母>/<full-identifier>/{orax.toml, logo.svg, README.md}`，如 `registry/o/ora-space.tavily-search/orax.toml`。经 GitHub API 确认根目录含 `{LICENSE, registry/}`，`registry/o` 下有 `ora-space.claude`、`ora-space.opencode`、`ora-space.skillhub`、`ora-space.tavily-search` 四个目录（来源：gh api repos/ora-space/marketplace/contents/registry/o）。

### Manifest（release 形态）

release manifest（`orax.toml`）字段：`resolver=1`、`identifier`（name 段）、可选 `title`、`namespace`、`kind`、`version`（semver）、`description`、可选 `homepage`/`license`、可选 `url`（release `.orax` 下载 URL）、可选 `sha256`、可选 `[head]{repository,branch}`、可选 `[dependencies]{ora}`、可选 kind 特异 `[workbench]`/`[webview]`。`RawPluginManifest`（release 形态，`deny_unknown_fields`）要求 `resolver:u64` 并把 name 段拼作 `identifier`；installed 形态 `RawInstalledManifest` 让 `resolver` 可选（默认 1）。样例 `ora-space.opencode`：`resolver=1,title="OpenCode",identifier="ora-space.opencode",namespace="official",kind="agent",version="0.2.2"`；样例 `ora-space.tavily-search`：`kind="mcp"`，`url` 指向 GitHub release `.orax` 并带匹配 `sha256`（来源：crates/plugin-manifest/src/manifest.rs:12、361-417）。

注意：resolver v1 只接受 namespace `official`，且只接受 kinds `{workbench,agent,webview,skill,mcp}`，其它一律 `Unsupported`（来源：crates/plugin-manifest/src/enums.rs:26-38、77-93）。

### 端到端发现/下载/安装/注册流程

1. `PluginManager::discover(data_dir)` 在 `<data-dir>/plugins/installed/<ns>/<name>/<version>` 下选最高 SemVer，读 `orax.toml`（bounded 字节数以防并发增长），跳过 symlink，坏包隔离为 discovery issue 不影响兄弟包（来源：crates/plugin-manager/src/discovery.rs:14-300）。
2. `RegistrySync::sync` 经注入的 gitlancer `Git` 对每个配置源做 clone/fetch/ff，`RegistryIndex::build_all` 扫描各 `checkout_dir/registry` 重建并原子替换 `registry_index.json`（来源：crates/plugin-registry/src/source.rs:40-138）。
3. `Installer::install`：`RegistryIndex::resolve_manifest_all` 从同步源 checkout 解析 release manifest 取 `url`/`sha256` → `HttpDownload` 下载 `.orax`（zip）并在下载中校验 `sha256`（缺失则 `MissingRelease`）→ `extract_archive` 带 `ExtractLimits` 解压到 `installed/<ns>/<name>/` 下临时 staging → `validate()`（entrypoint/kind 策略 + config.json 编译）→ 原子 rename 到 `<version>` 目录（已存在则 `AlreadyInstalled`）→ `scan_plugins + enable_plugin`（来源：crates/plugin-manager/src/install.rs:112-159、258-287；crates/backend/src/plugin.rs:489-542）。
4. 本地导入 `install_local`：从本地 `.orax` 解压到 disposable staging，读 archive 根的 `orax.toml`（`parse_installed`），校验自声明 `sha256`（能防损坏/传输损伤，但无防篡改保证），`validate` 后 rename，无需先 sync 市场，成功自动 enable（来源：crates/plugin-manager/src/install.rs:168-255）。

### plugin-sdk（作者侧）

`packages/plugin-sdk`（`@ora-space/plugin-sdk`）导出 `createPlugin`、`Plugin`、`PluginMethodError`、`HostRequestError`、`MethodHandler`、`NotificationHandler`、`EffectSurfaceDeclaration`、`createStorage/PluginStorage`、`defineAgent`、`defineWorkbenchPlugin`、`JsonValue` 等（`mod.ts` re-export `plugin.ts/agent.ts/storage.ts/workbench.ts/protocol.ts`）。SDK 用二进制 stdout 协议：4 字节 BE length + 1 字节 JSON-RPC frame type + UTF-8 JSON payload；帧 >16MiB 或畸形宿主消息会停止插件；`console.*` 重定向到 stderr（来源：packages/plugin-sdk/src/mod.ts:1-37、package.json、README.md）。

核心 `Plugin` API：`registerMethod(name,handler)`（`run()` 前注册，运行后不可变，重复/空名拒绝）、`declareEmit(name)` 白名单 plugin→host 通知（未声明则被宿主拒绝并终止进程）、`onNotification(name,handler)` 处理 host→plugin 通知（未处理只记录不致命）、`request(method,params,{timeoutMs})` 发送关联 JSON-RPC 请求（默认 30s 超时，`HostRequestError` kinds：`host data.kind`、`method_not_found`、`timeout`、`transport`）、`notify` 发送已声明通知、`declareEffectSurface` 声明 Skill 面、`run(transport)` 发 `ora/register` 并服务至 `ora/shutdown` 或 stdin EOF（来源：packages/plugin-sdk/src/plugin.ts:82-389）。

`defineAgent` 注册完整 agent 契约（`agent/start`、`stop`、`listModels`、`acp` 双向），`AGENT_NOT_INSTALLED=-32001` 是 ora 视为预期（安静重试）的码；可选 effects 声明 Skill 面 + `effect/waitForIdle`、`effect/restart` 协调方法；插件自己 spawn/拥有 agent CLI 进程，ora 从不触碰该进程 stdio（来源：packages/plugin-sdk/src/agent.ts:9-129）。`defineWorkbenchPlugin` 注册 page-callable methods，解包宿主 envelope `{surface:{instance_id,generation}, input}` 为 `WorkbenchCall`；v1 无 plugin→page 通道。`createStorage` 封装 `ora/storage/*`，逻辑斜杠分隔路径相对插件私有 data dir，拒绝绝对路径/`..`/symlink/`web-profile/`，read ≤8MiB（来源：packages/plugin-sdk/src/workbench.ts:41-68）。

### MCP-kind 编译

mcp 插件唯一 artifact 是 `assets/config.json`，由 `compile_configuration_file` 按 `transport` 成员是否存在分派：缺失 → Settings-only `CompiledDeclaration`；存在 → `CompiledMcpConfiguration`（`schemaVersion` 必须为 1）。`CompiledConfigurationFile = Settings(CompiledDeclaration) | Mcp(CompiledMcpConfiguration)`。`RawMcpConfiguration{schemaVersion, settings?, transport}`（camelCase、`deny_unknown_fields`）。Settings subset 拒绝 `secret`/`file`/`directory` 类型（phase 1 只支持 `string`/`number`/`boolean`）（来源：crates/plugin-config/src/mcp/mod.rs:34-209）。

`McpTransport` 是互斥枚举：`Stdio{command:PortableRelativePath under assets/, args:Vec<McpArgument>, env:BTreeMap}` 或 `Http{url:HTTPS Url(无 user/pass/fragment/query), headers:BTreeMap}`。`McpValueExpression = Literal(String) | Setting{id,prefix,suffix}`（必须引用已声明 Setting id）。stdio command 必须是 `assets/` 下无穿越的相对路径（PATH 查找如 npx/uvx 不可表达）；HTTP header 值必须是 Setting 引用（phase 1 禁止硬编码 API key）；HTTP URL 必须 HTTPS 无 query；env 名匹配 `^[A-Za-z_][A-Za-z0-9_]*$`（来源：crates/plugin-config/src/mcp/transport.rs:13-317）。

mcp 包校验：**不得** ship `main.js`（否则把代码偷渡进 config-only kind），**必须** ship `assets/config.json`（MCP 形态，恰好一个 transport），stdio transport 的 command 必须解析为包内真实可执行文件（Unix 查可执行位）。`InstalledMcpDescriptor{configuration:CompiledMcpConfiguration}` 是 validated contribution，仅是 install 时刻的静态合法性证明，不证明 Settings 已填或端点可达（来源：crates/plugin-manager/src/mcp.rs:13-128、validation.rs:132-158）。

## 5. Ora 项目规约

### 规约工作流

`specs/` 三层：`drafts/`（入口，当前空）、`changes/`（实现指引，编号匹配所修改 active 节）、`active/`（权威当前规格，按 feature 区编号文件）。新能力起于 draft，实现期移入 `changes/`（change spec 显式引用所修改的 active 节，如 `specs/changes/plugin/1-capability.md` 开头“本文件指导 specs/plugin/1-capability.md 中…的实现”），ship 时更新 `active/` 并移除 change spec。`specs/` 是只读评审面，`.md` 文件放入 `drafts/` 或 `changes/` 即自动出现在前端 Specs 子视图，无需注册步骤（来源：docs/spec-management.md:14-66）。

### Rust 规约

- crate 名前缀 `ora-`（如 `ora-core`、`ora-utils`、`ora-plugin-manifest`），workspace 列 24 crate + `apps/desktop/src-tauri` + `xtask`（来源：Cargo.toml:3-34；AGENTS.md）。
- 用 `ora-logging` 包装宏（`ora_trace!`/`ora_debug!`/`ora_info!`/`ora_warn!`/`ora_error!`）而非原生 `tracing`；用 `ora_logging::clock::now_local` 而非 `OffsetDateTime::now_local()`；`ora-logging` 拥有进程级 subscriber 设置、JSON 格式、sink、轮转、不可变进程时区（来源：docs/runtime-logging.md:6-11、73-78；AGENTS.md）。
- 通用、领域无关的逻辑放 `ora-utils`（`crates/utils`），不得依赖任何 `ora-*` crate、不得带领域词汇；路径/归档/安全校验应复用 `ora-utils::path` 与 `ora-utils::archive` 而非 crate-local 实现（来源：crates/utils/README.md:1-56；AGENTS.md）。
- 优先静态分发（generics + trait bounds）而非 trait object（`Box<dyn Trait>`）；用带关联数据的 enum 而非 optional 字段堆叠的 struct 使非法状态不可表达；无向后兼容，打破旧模式、删除 deprecated 代码（来源：AGENTS.md #4/#5/#6；docs/application-contracts-boundary.md:61）。
- 用本地时间而非 UTC；永不硬编码路径分隔符或手动拼接路径串，总用 `Path`/`PathBuf`/`.join()`（来源：AGENTS.md；docs/runtime-logging.md:52-53）。
- workspace lints 在 `Cargo.toml [workspace.lints.clippy]` 否认 `unwrap_used`、`expect_used`、`manual_unwrap_or`（及约 30 项其它如 `redundant_clone`、`uninlined_format_args`、`needless_borrow`），lint 闸门跑 `cargo clippy --workspace -- -D warnings`（**不**含 `--all-targets`）（来源：Cargo.toml:146-181；Taskfile.yml:114-119）。
- 每个函数签名上方加注释说明用途；解释 Why 不解释 What；复杂逻辑加内联注释；注释用英文（来源：AGENTS.md #1/#2）。
- 模块 README 规则：每个 `crates/` 下 crate 根有英文 `README.md`；每个 `src/` 下目录型生产模块有英文 `README.md`；单文件模块与 test/fixture 目录豁免；`crates/contracts`、`crates/domain`、`crates/pty` 是有意例外。README 记稳定事实（职责、非职责、公共边界、关键不变量、生命周期、失败语义、交互），本地实现理由/算法细节放代码注释（来源：AGENTS.md Module READMEs 节；find 输出）。

### 测试/lint 闸门

- `task test` = `task test:frontend` + `task test:crates`（长任务，迭代时用最小相关任务）。
- `task test:crates` 需 PATH 上有 rg+deno，先跑 `task lint:crates` 再 `cargo test --workspace`。
- `task lint:crates` = `cargo fmt --all --check` + `cargo clippy --workspace -- -D warnings`。
- `task format` = `scripts/format-changed.mjs`（仅改动的文件）。
- 前端测试在 `scripts/run-with-clean-stderr.mjs` 下运行，任何 stderr 上的 React Testing Library 警告（尤其 `not wrapped in act(...)`）都会让整个 `task test` 失败，即便 Vitest 报绿；TipTap/ProseMirror `setContent` 可触发 `flushSync`，约束见 AGENTS.md Tests 节。
- Rust 测试用 `pretty_assertions::assert_eq`、优先对整个对象做深度相等；测试 `tracing` 时装 test-scoped TRACE subscriber（`with_default`/`with_default`），优先用 `with_trace_logging`/`with_recorded_trace_logging` 共享 helper 覆盖所有 setup 与触碰 callsite 的代码；避免在测试中改进程 env（来源：AGENTS.md Tests 节；docs/runtime-logging.md:90-92）。
- 改动 add/change 行为时同步更新 `docs/`；新增 crate 或目录型模块时同改动加 README；改模块职责/边界/流程/交互时同改动更新对应 README（来源：AGENTS.md）。

### 设计研究笔记的位置

`docs/BRAINSTORM.md`（Goal/Why/Decision/Implementation status）是 pre-design 讨论/决策笔记的先例，位于 `docs/` 顶层。`specs/drafts/` 是 spec 形态草稿的下一步入口，不是探索性研究。多上下文 ADR 布局（`CONTEXT-MAP.md`、`docs/adr/`）在 `docs/agents/domain.md` 中有描述但当前不存在；domain.md 说若缺失就“proceed silently”，不应预先创建这些文件（来源：docs/BRAINSTORM.md:1-27；docs/agents/domain.md:1-58）。

## 6. RTK 工程分析

### rtk 做什么

rtk（Rust Token Killer）是一个高性能 CLI 代理，在 shell 命令输出到达 LLM 上下文前过滤/压缩，削减 60–90% 的 bash 输出字节。四种策略：智能过滤（去噪）、分组、截断、去重，按命令类型分别施加（ls/tree、cat/read、grep/rg、git status/diff/log、cargo test、pytest 等）（来源：rtk-develop/README.md:37-57、147-153）。

### 运行时形态

rtk 是单一 Rust 二进制，**单线程、无 async**（Cargo.toml 无 tokio/async-std/futures），目标启动 <10ms、内存 <5MB、二进制 <5MB；用 `anyhow::Result` + `.context()`、`LazyLock<Regex>`、失败回退（过滤失败 → 执行原命令）、子进程失败用 `std::process::exit(code)` 传播退出码；生产代码无 `unwrap()`（来源：rtk-develop/Cargo.toml:15-43、59-64；rtk-develop/CLAUDE.md:108-111；rtk-develop/.claude/rules/rust-patterns.md:9-145）。

`main.rs` 用 Clap `Commands` 枚举路由到 `src/cmds/*` 的 per-ecosystem filter 模块；token 节省记录在 SQLite（`src/core/tracking.rs`，token 估算为 bytes/4，无 tokenizer）；`rusqlite` bundled 依赖（来源：rtk-develop/CLAUDE.md:70；rtk-develop/Cargo.toml:33）。

### 分发

rtk 以 Rust 二进制分发：Homebrew、`curl install.sh`、`cargo install --git`、预构建 release（macOS/Linux/Windows）、`.deb`/`.rpm`（Cargo.toml 含 `cargo-deb`/`cargo-generate-rpm` 元数据）。rtk **本身不是 npm 包**。版本 0.42.4（来源：rtk-develop/README.md:68-100；rtk-develop/Cargo.toml:1-13、67-81）。

### rtk 的 hook 系统

rtk 的 hook 系统在**执行前改写命令**（不是输出拦截）：hook 调用 `rtk rewrite <cmd>` 把如 `git status` 映射成 `rtk git status`，随后 rtk 自身运行并过滤输出（来源：rtk-develop/hooks/README.md:13-31）。

所有改写逻辑集中在单一真相源 `src/discover/registry.rs::rewrite_command(cmd, excluded, transparent_prefixes) -> Option<String>`；hook 脚本与 TS 插件都是 shell 出 `rtk rewrite` 的薄委托（来源：rtk-develop/src/discover/registry.rs:569；rtk-develop/openclaw/index.ts:7-8；rtk-develop/hooks/claude/rtk-rewrite.sh:7-8；rtk-develop/hooks/opencode/rtk.ts:6-7）。

`rtk rewrite <cmd>` 是规范 hook 入口，带 4 值退出码协议：Exit 0+stdout=Allow（自动应用）；Exit 1=无 rtk 等价（passthrough）；Exit 2=Deny（阻断）；Exit 3+stdout=Ask（改写但需用户批准）。安全关键：`PermissionVerdict::Default` 必须映射到 exit 3（ask），绝不能 0，以免绕过最小权限（来源：rtk-develop/src/hooks/rewrite_cmd.rs:7-37、207-294；rtk-develop/openclaw/index.ts:10-17）。

`rtk hook <agent>` 子命令为 claude/cursor/gemini/copilot/droid/vibe 处理各 agent 特定 JSON（stdin 进、agent 特定 JSON 出）——原生二进制 hook 路径。`HookCommands` 枚举（Claude/Cursor/Gemini/Copilot/Droid/Vibe/Check）；各 `run_*()` 读 stdin、调 rewrite、格式化输出（来源：rtk-develop/src/main.rs:852-882、2435-2477；rtk-develop/src/hooks/hook_cmd.rs:61-854）。

复合命令处理：registry 处理 `&&`/`||`/`;`/`|`/`|&`/`&`，对 and/or/semicolon 两侧独立改写；管道保持 producer 原样；不可判定的构造（反引号/`$()`、heredoc、除 `2>&1` 外的文件重定向）强制 passthrough（来源：rtk-develop/hooks/README.md:226-235；rtk-develop/src/hooks/rewrite_cmd.rs:134-205）。覆盖控制：`RTK_DISABLED=1` 单命令禁用、`config.toml [hooks] exclude_commands`、已是 rtk 则 passthrough（无 `rtk rtk`）（来源：rtk-develop/README.md:410-428）。

### Per-tool 集成（hooks/）

`hooks/` 下的 per-agent 集成是薄委托（shell 脚本、TS/Python 插件、规则文件），**不**含任何过滤逻辑；它们只解析 agent JSON 并调 `rtk rewrite`。三层：Full hook（shell/Rust 二进制）、Plugin（TS/Python）、Rules file（prompt 级）。覆盖 claude、copilot、cursor、cline、windsurf、codex、opencode、hermes、pi、vibe（来源：rtk-develop/hooks/README.md:1-63、268-275）。

- Claude Code hook（`hooks/claude/rtk-rewrite.sh`）：bash PreToolUse hook，需 `jq`；委托 `rtk rewrite`，emit `hookSpecificOutput` 含 `updatedInput` 与 `permissionDecision` allow/ask；所有失败路径 exit 0；版本守卫缓存在 `$XDG_CACHE_HOME/rtk-hook-version-ok`（来源：rtk-develop/hooks/claude/rtk-rewrite.sh:1-101）。
- OpenCode hook（`hooks/opencode/rtk.ts`）：TS 插件用 `zx`，经 `tool.execute.before` 事件原地改写 `args.command`；检查 `which rtk`；失败静默（来源：rtk-develop/hooks/opencode/rtk.ts:1-39）。
- hooks 契约：hook **永不**阻断命令执行，所有错误路径（缺二进制、坏 JSON、rewrite 崩溃）exit 0/passthrough（来源：rtk-develop/hooks/README.md:243-261）。

### openclaw 插件先例（rtk 作为插件的参考打包）

openclaw 插件是 rtk 已有的“作为插件”打包先例，是一个**薄 npm 包**（`@rtk-ai/rtk-rewrite`），`package.json` `main=index.ts`，`files=[index.ts, openclaw.plugin.json, README.md]`，**不**含编译后 JS，也**不**捆绑 rtk 二进制——它 shell 出 PATH 上的 `rtk` 二进制；rtk 须单独安装。安装方式：复制到 `~/.openclaw/extensions/rtk-rewrite/` 或 `openclaw plugins install ./openclaw`（来源：rtk-develop/openclaw/package.json:1-26；rtk-develop/openclaw/README.md:13-40）。

manifest（`openclaw.plugin.json`）字段：`id`、`name`、`version`、`description`、`homepage`、`license`、`configSchema`（JSON-schema 对象，含 `enabled`/`verbose` 布尔属性 + defaults + descriptions）、`uiHints`（每 config key 的 label 字符串）。manifest 中**无** `entry`/`main` 字段——入口由 `package.json` `main=index.ts` + OpenClaw 文件命名约定隐含（来源：rtk-develop/openclaw/openclaw.plugin.json:1-28）。

入口（`index.ts`）default-export 一个 `register(api)` 函数，订阅 `before_tool_call`（priority 10），只拦截 `exec` 工具调用：检查 `which rtk`，`execFileSync('rtk', ['rewrite', command], {timeout:2000})`，按退出码返回 `{params:{...event.params, command:rewritten}}`（allow）、`{block:true}`（deny）、或带 `requireApproval` 的 ask。失败静默 passthrough（来源：rtk-develop/openclaw/index.ts:75-157）。

## 7. 评审后确认的设计

后续设计评审推翻了“Hook 是 Deno 进程、通过 `main.js` 调用 RTK”的早期假设。RTK Hook Plugin 是一个新的、无进程的 `hook` kind：包内携带一个目标平台的 RTK 可执行文件，并用不可变、强类型的 Hook Configuration 描述它。Agent Plugin 如何消费 Hook 不属于首个安装闭环。

### 首个闭环

- 支持 marketplace 发现、target 选择、SHA-256 校验、安全解压、Hook 静态校验、安装、自动全局启用、禁用和卸载。
- 首发身份为 `official/rtk-ai.rtk`，Plugin 版本 `0.1.0`，内嵌 RTK `0.45.0`。
- 首发 target 只有 `x86_64-pc-windows-msvc`；resolver 1 已有 `[[targets]]` 规格，代码补齐实现即可。
- `.orax` 携带 `orax.toml`、`assets/config.json`、`assets/rtk.exe`、`LICENSE`、`README.md`、`logo.svg`，不含 `main.js`。
- 包装仓库为 `ora-space/rtk-hook-plugin`；CI 固定并校验上游 RTK release SHA-256，生成单 target `.orax`，执行 `--version`、rewrite 协议和临时 PATH smoke test，再发布 release。
- marketplace 条目位于 `registry/r/rtk-ai.rtk/`，只引用已经存在且已验证的 release asset。

### 配置分层

`orax.toml` 只负责身份、kind、版本、release 选择和 installed artifact target。Hook 的不可变 contribution 位于必需的 `assets/config.json`：

```json
{
  "schemaVersion": 1,
  "hook": {
    "protocol": "rtk-rewrite-v1",
    "executable": "assets/rtk.exe",
    "command": "rtk",
    "toolVersion": "0.45.0"
  }
}
```

RTK v0.1.0 不声明用户 Settings。未来确有可消费的、插件全局、非敏感标量配置时，可增加同级 `settings`；任意 JSON、Agent 配置模板、数组伪装字符串都不允许。Settings-only、MCP `transport`、Hook `hook` 三种形态必须严格互斥。

### 安全与生命周期边界

- plugin-manager 只做静态校验，安装期间绝不执行 payload；可运行性由包装 CI 和隔离 E2E 证明。
- Hook 必须无 `main.js`，executable 必须是包内非 symlink 普通文件，installed artifact target 必须精确匹配宿主；Windows 首版要求 `.exe`。
- 安装后沿用现有默认自动启用行为。命令别名与另一 Enabled Hook 冲突时，安装保留但新插件保持 disabled，并返回强类型原因。
- 本次不实现 Workspace 选择、Agent Plugin 交付、PATH 注入或 agent 配置。未来进入消费阶段后，disable/uninstall/upgrade 必须先协调消费者 idle/restart，不能直接删除仍被使用的 payload。
- RTK v0.45.0 无法真正关闭 tracking；未来消费时必须将 DB 重定向到 Ora 管理目录、关闭 tee 和 telemetry，并在用户文档中披露本地原始命令与项目路径的 90 天记录行为。

### 验收

除 Rust/TypeScript 单元与集成测试外，必须使用干净 Ora 数据目录在 Windows x86_64 完成：同步 marketplace、显示 compatible、下载、摘要校验、安装、自动启用、禁用、卸载；隔离 E2E 还必须从实际 installed path 验证 RTK `0.45.0` 和 `rtk-rewrite-v1`，并仅在测试进程内临时扩展 PATH。

## 附录 A：评审前的过时备选分析

以下内容保留为探索记录，但不再是实现建议。凡是涉及 Hook `main.js`、Deno runtime、runtime registration、Hook Settings 或要求系统预装 RTK 的描述，均由上面的确认设计取代。

### 关键差距分析：把 rtk 打包为 ora hook 插件

### ora 有 hook 插件类型吗？（代码 + 规约双重裁定，调和）

**代码裁定（§2）**：`PluginKind` 是封闭 5 变体枚举（`Workbench/Agent/Webview/Skill/Mcp`），`FromStr` 拒绝其它值（来源：crates/plugin-manifest/src/enums.rs:47-93）。`PluginContribution`、`InstalledPluginContribution`、`validate_registration` 均按这 5 kind 分派，无 Hook 变体。

**规约裁定（§3）**：specs/active/plugin/ 定义 kind 为 `agent/mcp/workbench/webview`（specs 文本未单列 skill kind，但代码有），对 `hook` 全树 grep 零匹配。能力模型以 `[permissions.*]` 共享分类法扩展，明确不以新 kind 表达新能力。

**调和结论**：**ora 当前没有 hook 插件 kind，也没有 hook 能力。** 代码与规约一致。rtk 作为“命令执行前改写、执行后过滤”的代理，无法直接套进任何现成 kind：

- `agent` 是结构上最接近的（Deno 子进程进程型插件，能注册 methods + emit notifications），但语义上 agent 插件是**提供**一个 agent（经 `agent/acp` 说 ACP），自身是 agent provider 身份；rtk 不提供 agent，只拦截/代理既有工具执行路径。强行套 `agent` 需让 rtk 实现 `agent/start|stop|listModels` 并持有一个它概念上不拥有的 `AgentRef` 身份。
- `mcp` 纯配置无进程，rtk 是带自有进程与过滤逻辑的代理，不符。
- `workbench` 是 page-bridged 进程；`webview` 是外部 HTTPS 站点；`skill` 是静态资产包。都不匹配 hook/代理。

因此把 rtk 打包为 ora hook 插件，需要新增一个 `hook` 类型/能力，端到端贯穿 manifest、manager、contracts、lifecycle、registration 各层。代码侧最小改动面：`crates/plugin-manifest/src/enums.rs:49-55` 加 `PluginKind::Hook` 变体；`crates/plugin-manager/src/validation.rs:23-29` 加 `PluginContribution::Hook(...)`；`crates/contracts/src/plugin.rs:19-36` 加 `InstalledPluginContribution::Hook`；`crates/plugin-lifecycle/src/registration.rs:24-57` 加 Hook arm；`crates/plugin-lifecycle/src/permissions.rs` 加 Deno 权限集；manifest 加 kind 特异节（类比 `[workbench]`/`[webview]`）。运行时/launch 路径（Deno 子进程 + Job-Object 进程树）已足够通用，能托管这样一个进程型插件，主要工作在 kind 枚举 + contribution + manifest 节 + 宿主侧 hook 协议（类比 `agent/acp`）。

### 若选“新增 hook 能力”路径：rtk 的 manifest/runtime/lifecycle/settings 形态

**Manifest（`orax.toml`）**：`resolver=1`、`identifier`（如 `ora-space.rtk-rewrite`）、`namespace="official"`（当前 v1 只接受 official；来源：crates/plugin-manifest/src/enums.rs:26-38）、`kind="hook"`（新增）、`version`、`description`、`homepage`、`license`、`url`（`.orax` 下载 URL）、`sha256`、可选 `[head]`/`[dependencies]`、新增 `[hook]` 节（类比 `[workbench.methods]`，声明可订阅的 hook 事件封闭集合，如 `tool/pre-exec`、`tool/post-exec`、`session/pre-send`）、`[permissions.process]`（rtk 需 `executables` 含 `rtk`）。

**Runtime**：进程型插件（Deno 子进程跑 `main.js`，用 `@ora-space/plugin-sdk` 的 `createPlugin()`），经 `ora/register` 声明 methods（宿主可调，如 `hook/rewrite`）与 emits（plugin→host 通知，如 `hook/rewritten`、`hook/denied`、`hook/ask`）。`main.js` 调 `rtk rewrite` 子进程，把 4 值退出码协议映射到 ora 的 hook 返回形态。

**Lifecycle**：`PluginLifecycle` 的 `enable_plugin` 对 Hook kind 也会启动进程（与 Agent/Workbench 类似）；`complete_launch` 建 data dir、派生 Deno 权限、调 launcher.launch、`validate_registration`（需新增 Hook arm）、置 Running、spawn 通知泵 + 退出监视。`PluginGenerationLease` 保证 hook 随其进程 generation 一起消亡。

**Settings**：`assets/config.json`（Settings 形态）暴露 `enabled`（bool, default true）、`verbose`（bool, default false）、`exclude_commands`（数组），对应 rtk 的 `config.toml [hooks]`。ora 的 `CompiledDeclaration` 已支持 string/number/boolean（phase 1）。

### 若选“扩展现有 kind”路径：哪个最贴近？

若不新增 kind，最贴近的是 `agent`（进程型、可注册 methods + emits、有 start/stop 生命周期）或 `workbench`（已有 host→main.js invoke + manifest 方法表 ∩ runtime 注册方法交集的 allowlist 模式）。但 `agent` 要求 rtk 假装是一个 agent provider，语义错配严重；`workbench` 的 page-bridge 模型对 rtk 无意义。两者都是“勉强复用”，不如新增一个语义干净的 `hook` kind。

### openclaw.plugin.json vs ora manifest 字段级 delta

| 字段                  | openclaw.plugin.json                                     | ora `orax.toml`（release 形态）                                                                             | 差异                                                                                                          |
| --------------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| id                    | `id`（如 `rtk-rewrite`）                                 | `identifier`（如 `ora-space.rtk-rewrite`，含 namespace 段）                                                 | ora 用 `namespace.identifier` 复合 id；namespace 当前只接受 `official`                                        |
| name                  | `name`                                                   | `title`（可选）                                                                                             | ora 把 name 段拆进 `identifier`，展示名用 `title`                                                             |
| version               | `version`                                                | `version`（semver）                                                                                         | 一致                                                                                                          |
| description           | `description`                                            | `description`                                                                                               | 一致                                                                                                          |
| homepage              | `homepage`                                               | `homepage`（可选）                                                                                          | 一致                                                                                                          |
| license               | `license`                                                | `license`（可选）                                                                                           | 一致                                                                                                          |
| entry                 | **无**（由 `package.json main=index.ts` 隐含）           | **固定 `main.js`**（agent/workbench）；mcp 禁止 `main.js`                                                   | ora 要求固定 entrypoint 文件名 `main.js`，openclaw 无显式 entry 字段                                          |
| configSchema          | `configSchema`（JSON-schema 对象，含 enabled/verbose）   | ora 用 `assets/config.json` 的 Settings subset（string/number/boolean；phase 1 拒绝 secret/file/directory） | 机制不同：openclaw 用 manifest 内嵌 JSON-schema；ora 用独立 `assets/config.json` 编译为 `CompiledDeclaration` |
| uiHints               | `uiHints`（每 key 的 label）                             | ora 无对应字段（settings 描述由 `assets/config.json` 内 schema 提供）                                       | ora 无 manifest 级 uiHints                                                                                    |
| kind                  | 无（OpenClaw 无 kind 分类）                              | `kind`（必须，封闭枚举）                                                                                    | ora 强制 kind；hook 不存在                                                                                    |
| url/sha256            | 无（npm 安装）                                           | `url`（HTTPS 下载 .orax）+ `sha256`（hex）                                                                  | ora 是 .orax zip + 校验和分发，非 npm                                                                         |
| [head]/[dependencies] | 无                                                       | 可选 `[head]{repository,branch}`、`[dependencies]{ora}`                                                     | ora 有 git head 与 ora 版本依赖声明                                                                           |
| hook events           | 隐含（`api.on("before_tool_call", ...)` 在 index.ts 内） | 需新增 `[hook.events]` 封闭集合节                                                                           | ora 要求 manifest 声明可订阅事件，运行时与注册方法取交集（workbench 模式）                                    |

### rtk 打包面对 ora 的形态

rtk 的承重产物是 Rust 二进制（`rtk`，v0.42.4）在 PATH 上，以 Homebrew/cargo install/预构建 release/.deb/.rpm 分发，**不是 npm 包**。openclaw 先例表明“rtk 作为插件”是**独立的薄包**（npm `@rtk-ai/rtk-rewrite`），唯一职责是声明 manifest + shell 出 `rtk` 二进制，**不**捆绑 Rust 代码、**不**捆绑 rtk 二进制。

对 ora，rtk 的 ora 插件包应是类似薄委托：一个 `.orax` zip，含 `orax.toml`（`kind="hook"` 或所选 kind）+ `main.js`（Deno 进程胶水）+ `assets/config.json`（settings）+ 可选 `assets/`（若要捆绑 rtk 二进制，但 ora 的 `McpTransport` stdio 路径要求 command 在 `assets/` 下且是包内可执行文件——可作参考）。`main.js` 用 `createPlugin().registerMethod("hook/rewrite", ...)`，内部 `Deno.run`/`Deno.Command` 调 `rtk rewrite`，把 4 值退出码映射为 ora hook 返回（allow/passthrough/deny/ask）。rtk 二进制可（a）要求预装在 PATH（openclaw 模型，最低摩擦），或（b）捆绑进 `.orax` 的 `assets/`（需 ora 支持 hook kind 下 `assets/` 可执行文件，类似 mcp stdio command 的 contained+executable 校验）。

## 附录 B：评审前的过时建议与问题

### 设计建议（尚不实现，先起草）

1. **先写研究/决策笔记**于 `docs/`（如 `docs/rtk-hook-plugin-research.md`），沿用 `docs/BRAINSTORM.md` 的 Goal/Why/Decision/Implementation status 形态；`specs/drafts/` 是 spec 形态草稿的下一步，不是探索性研究。
2. **起草 draft spec** 于 `specs/drafts/plugin/hook.md`（或编号 `8-hook.md`），参考 `specs/active/plugin/{0-overview,1-capability,6-workbench,7-webview}.md` 的中文设计文档体例，至少定义：
   - hook 点封闭枚举（如 `session/start`、`session/stop`、`turn/pre`、`turn/post`、`tool/pre-exec`、`tool/post-exec`、`message/pre-send`、`message/post-receive`），用 typed enum 而非任意字符串（镜像 workbench.methods 封闭集与 webview DownloadAction 封闭集）。
   - 命令/消息拦截模型：hook 是观察者（fire-and-forget，不可变）还是拦截器（可 approve/deny/modify），envelope 形态遵循宿主持有身份（confused-deputy 防护，workbench §6.270-277）——hook 收到 `{surface/hook 上下文, input}`，绝不含 `plugin_id`/`version` 作为可路由字段。
   - 输出过滤管线（若 hook 过滤 agent 输出）：排序（多 hook 按声明序，如 webview download first-match）、幂等、大小限制（复用 1 MiB/16 MiB）、错误分类法（复用 workbench 错误码模式）。
   - 注册与 generation 模型：`orax.toml [hook.events]` 封闭列表 ∩ `main.js ora/register.params.events` 运行时注册，镜像 workbench 的 `effective_methods` 交集；`PluginGenerationLease` 使 hook 随进程 generation 消亡。
   - 安全边界：零权限 Deno、宿主持身份、不绕过 agent/mcp/workbench 权限模型、新增 `[permissions.hook]` 强类型配置（依 1-capability.md:216）。
   - 生命周期/install 不变量：installed/ 不可变、data/ 存状态、版本级授权、与 Effect reconcile 系统交互（不绕过 `AgentTarget` WaitingForIdle/Quiescing，effect/2-declaration.md:148-168）。
3. **实现期移入 `specs/changes/plugin/8-hook.md`**，按 `specs/changes/plugin/1-capability.md` 模板（目标/存储布局/Manifest 权限/SDK 与宿主行为/生命周期/验收标准），声明实现范围与验收标准（含真实 Tauri/runtime 集成测试，非仅 SDK 单测，依 6-workbench.md:446、7-webview.md:369）。ship 时更新 `specs/active/plugin/` 并移除 change spec。
4. **代码侧原型**：在 `crates/plugin-manifest/src/enums.rs` 加 `PluginKind::Hook`；`crates/plugin-manager/src/validation.rs` 加 `PluginContribution::Hook(...)` + `validate` Hook arm；`crates/contracts/src/plugin.rs` 加 `InstalledPluginContribution::Hook`；`crates/plugin-lifecycle/src/registration.rs` 加 Hook arm；`crates/plugin-lifecycle/src/permissions.rs` 加 Hook 的 Deno 权限集；在 `packages/plugin-sdk` 加 `defineHookPlugin`（类比 `defineWorkbenchPlugin`）。先做最小原型：一个 hook kind 插件，注册 `hook/rewrite` method，宿主在 Bash/exec 工具 pre-exec 调用之，`main.js` 调 `rtk rewrite` 并返回改写后命令/allow/deny/ask。
5. **README/文档同步**：新增 crate 或目录型模块时同改动加 README（英文）；改 `docs/agent-runtime.md`、`docs/application-contracts-boundary.md` 等若行为触及已文档化区域。
6. **lint/test 闸门**：迭代用 `task test:crates`（需 rg+deno），完成前跑 `task test`（含前端 stderr-clean 闸门）；`task format` 收尾；Rust 测试用 `pretty_assertions` + `with_trace_logging`。

### 待用户拍板的问题

1. **hook 应是新 plugin kind，还是现有 kind（agent/workbench）的扩展，还是根本不是插件模型、而是内置 effect/skill？** —— 代码与规约均无 hook，新增 kind 是干净路径但工作量大；复用 agent/workbench 语义错配；把 rtk 做成内置 effect/skill 则绕过插件分发链路。需用户决策方向。
2. **rtk 以 Rust 二进制分发（要求 PATH 预装），还是由 ora 插件包捆绑 rtk 二进制？** —— openclaw 模型是薄委托不捆绑；ora 的 mcp stdio 路径要求 command 在 `assets/` 下且包内可执行，提供了“捆绑二进制”的先例参考，但 hook kind 是否允许 `assets/` 下可执行文件需新规约确认。
3. **ora hook 层支持执行前改写命令（rtk 模型），还是仅执行后输出过滤？** —— rtk 的 hook 是 pre-execution rewrite；若 ora 只提供 post-execution hook，集成形态根本改变（rtk 无法以“改写命令→自身执行过滤”模型工作，需改为纯输出后处理）。
4. **ora 的 hook 事件名、返回形态（如何表达 allow/deny/ask、如何改写工具 input）、config-schema 机制**须对照 ora-plugin-protocol 确认（openclaw.plugin.json 不可照抄）。调研中 SDK 的 `defineUiPlugin` 在 README 出现但 `mod.ts` 未导出、SDK 版本 package.json 0.2.0 与 commit `46601fc5` 的 0.3.0 不一致，均需确认。
5. **ora 是否需要新增 `rtk hook <ora-agent>` 子命令（原生二进制 hook 路径，如 claude/gemini/vibe），还是用薄委托 shell 出 `rtk rewrite`？** —— 前者避免每次调用子进程开销但需在 rtk 的 `HookCommands` 与 `hook_cmd.rs` 加 ora 变体；后者低摩擦、与 openclaw/opencode/hermes 一致。
6. **namespace 限制：当前 resolver v1 只接受 `official`（来源：crates/plugin-manifest/src/enums.rs:26-38），rtk 作为第三方能否发布？** —— 需确认是否放宽 namespace 或由 ora 官方收录。
7. **rtk 需要的 hook 协议方法是宿主可调 methods、plugin-emitted notifications，还是两者皆要？** —— 若 modeled as new Hook kind，`validate_registration` 的 Hook arm 需明确；rtk 可能还需一个类似 `agent/exited` 的 exited/error notification（plugin-agent-runtime.md §12.5 提到的未决项）。
8. **bundled vs third-party 信任级别**（plugin-agent-runtime.md §12.4 当前“无差别”）对 rtk 的影响：rtk 会带较宽 Deno 权限（`--allow-run`/`--allow-read`/`--allow-env`/`--allow-net`，可比 agent 插件），需确认 bundled 信任是否需提升。
