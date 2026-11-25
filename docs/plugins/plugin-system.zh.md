# 插件系统

[English](plugin-system.md) | 中文

插件是扩展 Ora 的方式：Agent、Skill、MCP server、UI surface、Hook、工作流文档，以及由其他插件组成的扩展包，都以已安装的 `.orax` 包的形式到来。本文介绍包模型、进程生命周期、宿主服务和配置。包从哪里来见 [Marketplace](marketplace.zh.md)；UI surface 见 [Surface](surfaces.zh.md)；插件日志见[插件日志](plugin-logging.zh.md)。

| Crate                         | 负责                                                 |
| ----------------------------- | ---------------------------------------------------- |
| `ora-plugin-manifest`         | 解析和校验 `orax.toml`                               |
| `ora-plugin-manager`          | 发现已安装的包、宿主侧的包校验、安装、更新、本地导入 |
| `ora-plugin-lifecycle`        | 插件进程的唯一所有者；宿主请求服务；运行时状态       |
| `ora-plugin-runtime`          | 启动 Deno、stdio 协议、注册、调用和通知              |
| `ora-plugin-protocol`         | 与 `packages/plugin-sdk` 共享的线上契约              |
| `ora-plugin-config`           | 设置声明、MCP 和 Hook 配置形状、已存的值             |
| `ora-plugin-registry`         | marketplace 源同步和可用插件索引                     |
| `ora-backend`（`src/plugin`） | 组装：操作、扩展包、Hook 生命周期、marketplace 同步  |

## 身份与目录布局

插件以 `ora_domain::PluginId` 标识，在所有契约上写作 `<namespace>/<name>`。manifest 从不写自己的命名空间：宿主根据包的安装来源 marketplace 源推导它（导入的归档则是 `local`），因为命名空间决定了一个包拥有哪个安装目录、数据目录和哪些 Skill 行。

```text
~/.ora/plugins/
  installed/<namespace>/<name>/<version>/   只读的包（orax.toml、main.js、assets/、logo.svg）
  data/<namespace>/<name>/                  插件数据：store.json、ora/storage 根目录、下载、web profile
  logs/<namespace>/<name>/                  宿主管理的插件日志，见插件日志
  sources/                                  marketplace 源的 checkout
  cache/                                    registry_index.json 和下载中的 .orax
  log-levels.json                           每个插件的日志级别
```

发现过程扫描 `installed/`，把每个版本目录按 SemVer 解析，每个插件只选最高版本；该版本无效时不回退。命名空间和 identifier 来自目录层级，并且必须与 manifest 一致；格式错误的包成为结构化的发现问题，不进入快照。只有显式扫描才会重建快照。

## 种类

| 种类        | 贡献                                                                                       | 进程                                                                  |
| ----------- | ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------- |
| `agent`     | 恰好一个 Agent，以包的插件 id 标识，见 [Agent 运行时](../agents/agent-runtime.zh.md)       | Deno `main.js`，权限较宽，可以启动宿主管理的子进程                    |
| `workbench` | 随包附带、作为 surface 显示的页面                                                          | Deno `main.js`，没有任何 `--allow-*` 权限，由页面的第一次桥接调用启动 |
| `webview`   | 作为 surface 显示的外部 HTTPS 站点                                                         | 无                                                                    |
| `skill`     | `assets/<name>/SKILL.md` 下的一个或多个 Skill                                              | 无                                                                    |
| `mcp`       | 来自 `assets/config.json` 的一个 MCP server，见 [Session MCP](../agents/session-mcp.zh.md) | 无（server 由 Agent 运行）                                            |
| `hook`      | 带 `init`/`deinit` 生命周期命令的原生可执行文件                                            | 无（宿主只在获得授权时运行命令）                                      |
| `workflow`  | 导入工作流库的工作流文档，见[工作流包](workflow-packages.zh.md)                            | 无                                                                    |
| `pack`      | 同一来源中其他插件的列表，见 [Marketplace](marketplace.zh.md#扩展包)                       | 无                                                                    |

manifest crate 检查形状（字段类型、identifier 语法、`[webview]` 恰好出现在 webview 插件上、`[workbench]` 只出现在 workbench 插件上），并忽略未知字段，这样增量的 schema 变化不会让旧客户端丢掉列表条目；破坏性变化由 `resolver` 把关。manager 补充依赖磁盘上的包的检查：`main.js` 恰好存在于 agent 和 workbench 包中；workbench 附带 `assets/index.html`；webview 的 `start_url` 被 `allowed_origins` 覆盖；MCP 的 stdio 命令解析为包内的普通文件；Hook 可执行文件解析为 `assets/` 下非符号链接的普通文件。

一个发布要么是一个通用的 `url` + `sha256`，要么是一组 `[[targets]]`，每个带精确的 Rust target triple。只有附带原生二进制的种类允许按目标发布：`hook`（它本身就是那个二进制）和 `agent`（可以附带它的 CLI）。按目标安装的包带有声明其目标的 `[artifact]` 一节，在线安装和本地导入都会与宿主比对；目标从不属于身份的一部分。Ora 不再支持 macOS（v0.6.0 是最后一个 macOS 版本），所以 `*-apple-darwin` triple 已退役：清单里的 darwin 条目会被跳过而不是让整个清单失败，只有 darwin 条目的清单没有可安装的发布，已安装包的 `[artifact]` 声明 darwin triple 时会被拒绝。

## 生命周期

`ora-plugin-lifecycle` 是插件进程的唯一所有者，所以设置里显示的运行时状态总是对应真实存在的进程。每个已安装的插件都可用；agent 和 workbench 进程按需启动，停止它们不改变可用性；其他种类没有进程，保持 `stopped`。生命周期打开时所有插件都处于停止状态。

- 需要说协议的使用方是连接而不是启动：`ensure_running` 激活停止或失败的插件并有界地等待，`connection` 返回固定在某一代上的 `PluginConnection`，所以重启后的插件从不会通过过期的连接被访问。
- 每次启动都运行一个退出监视器和一个通知泵。如果进程还活着但通知超过短暂的宽限仍然停止，这次启动会被标为失败。
- 每个插件的操作在它自己的锁上串行化；不相关的插件独立进行。扫描按稳定顺序获取所有锁，以返回一个一致的快照。
- 卸载先关闭插件的 surface，停止进程，再把 `installed/<namespace>/<name>`（以及请求时的数据和日志目录）暂存起来，然后提交删除；暂存失败会回滚已移动的内容。删除数据的情况见[插件日志](plugin-logging.zh.md#生命周期)。

### 启动

生命周期创建 `data/<namespace>/<name>/`（含 `downloads/`），按种类推导 Deno 权限，并以包根目录为 cwd 启动 `main.js`。不注入任何环境变量；插件不知道任何宿主路径。握手之后按种类校验注册：workbench 插件可以暴露方法，但不能声明发出的通知；agent 的契约由 Agent 运行时验证。

## 协议与 SDK

`crates/plugin-protocol` 拥有 Rust 宿主、测试插件和 `packages/plugin-sdk` 共享的线上契约：分帧、JSON-RPC 方法常量、注册，以及 Agent 控制、存储、子进程和 workbench 调用的 DTO。`task export-contracts` 把生成的绑定写入 `packages/plugin-sdk/src/protocol`（`constants.ts` 和 DTO 模块）；`index.ts`、`json.ts` 和 `transport.ts` 保持手写。不要在 SDK 里重复线上名称：加到 `ora-plugin-protocol` 后重新生成。

stdout 只承载协议帧：四字节大端长度（含类型字节）、一字节帧类型和 UTF-8 JSON 载荷，最大 16 MiB。协议故障会让进程作废。多词的路径片段用 snake_case（`agent/list_models`、`ora/childprocess/close_stdin`）。stderr 承载诊断信息，见[插件日志](plugin-logging.zh.md)。

插件在 `ora/register` 中一次性声明两个方向的流量：

```jsonc
{
  "method": "ora/register",
  "params": {
    "methods": ["agent/start", "agent/stop", "agent/list_models"],
    "emits": ["agent/acp"],
  },
}
```

宿主只能调用列出的 `methods`；`emits` 之外的通知会让连接作废。插件发给宿主的请求不需要声明。仍在 `ora/register` 中附带 `effectResources` 的旧版 Agent 插件仍可连接成功：宿主忽略该遗留字段，且从不调用 `effect/*`。

`agent/discover` 接收 `AgentDiscoverParams { cwd, includeCommands, deadline }`：这次发现针对的 Workspace，调用方是否需要 Agent 的命令，以及宿主等到哪一刻（Unix 毫秒）。宿主的超时是同一时刻，排队写出的时间也算在内；等待和清理都以它为准。它返回模型，知道时还返回命令；命令列表缺省表示不知道，空列表表示 Agent 没有命令。格式错误的命令逐条丢弃。宿主按需调用它，预算比普通控制调用更长。

`agent/list_models` 是只返回模型的旧名。Agent 插件至少要注册两者之一；注册了 `agent/discover` 时宿主只调用它，不会两个都调，失败也不退回。SDK 两个都注册，所以旧宿主仍能列出模型。

### 宿主服务

启动时为插件绑定一个请求处理器，所以插件身份来自启动本身，从不来自请求参数。

- **存储**（`ora/storage/list`、`read`、`write`、`remove`）在相对于插件数据目录的逻辑路径上工作。路径是 `PortableRelativePath`，解析时不经过任何符号链接；宿主自有的 `web-profile/` 子树被拒绝。文件上限 8 MiB，因为 base64 必须装进一帧。错误带有 `data.kind`（`invalid_params`、`invalid_path`、`not_found`、`too_large`、`io`）。
- **子进程**（`ora/childprocess/spawn`、`write`、`close_stdin`、`kill`，仅限 agent 插件）让插件把子进程交给宿主拥有，享受整树终止。`spawn` 接收 `command`（由操作系统解析）或 `packageCommand`（拼接到插件安装根目录上，且必须留在其中）。`package_command_missing` 表示包里那个路径什么都没有，这是插件唯一应当回退到 `PATH` 的失败；SDK 的 `spawnAgentProcess` 实现了这一点。宿主推送 `ora/childprocess/stdout`、`stderr` 和 `exit` 通知，exit 从不早于最后一块输出；某一代退出时，它的所有子进程都会被杀掉。

## 操作

| 操作                                                                            | 用途                                                            |
| ------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `listInstalledPlugins`、`scanPlugins`                                           | 已安装快照；显式重新扫描                                        |
| `installPlugin`、`updatePlugin`、`importPlugin`                                 | 从 marketplace 安装或更新；把本地 `.orax` 导入 `local` 命名空间 |
| `activatePlugin`、`stopPlugin`、`uninstallPlugin`                               | 进程控制和卸载（可选择删除数据）                                |
| `getPluginConfiguration`、`savePluginConfiguration`、`resetPluginConfiguration` | 插件设置，见下文                                                |
| `listHookLifecycleReports`、`initializeHook`                                    | Hook 生命周期结果；用户请求的 `init`                            |
| `getPluginLogLevel`、`setPluginLogLevel`                                        | 见[插件日志](plugin-logging.zh.md)                              |
| `listMcpHealth`、`probeMcpHealth`                                               | 见 [Session MCP](../agents/session-mcp.zh.md#宿主-mcp-健康)     |

安装时下载 `.orax`（边下载边校验 SHA-256），安全地解压到暂存目录，再用一次重命名提交到 `installed/<namespace>/<name>/<version>`。校验依据的是归档内的 manifest，所以已安装的包天然可以被发现；它的 `identifier`、`version` 和 `kind` 必须与列表条目一致，描述性元数据可以不同，以包为准。更新在下载之前拒绝 no-op 和降级，然后退役其他所有版本。安装、更新和卸载会重新协调 Agent 集合，并发出 `PluginStatusChanged`。

## 配置

包可以在 `assets/config.json` 中声明配置，它有四种互斥的形状：v1 的纯设置；MCP 配置（设置加恰好一个 `stdio` 或 `http` 传输）；Hook 配置（设置加一个 `hook` 描述符）；或 v2 Agent 声明（`agent.modelAccess` 加可选 Settings）。混合 contribution 键的文件按失败处理；MCP 和 Hook 包各自要求对应形状，只有 Agent 包可以携带 Agent 形状。

v1 形态已存的值是插件全局的，与安装的版本无关，存在 `data/<namespace>/<name>/store.json` 中。保存时原子地替换完整的覆盖集合，并使用乐观的 revision 加声明指纹，所以过期的编辑器不能覆盖较新的状态。恢复时格式错误的存储会以备份名保留。声明无效的包仍然可见但无法启动。列表响应包含每个插件的配置完整性。MCP 设置只支持 `string`、`number` 和 `boolean`；HTTP header 必须通过设置引用绑定，所以凭据不能写死在包里。

[Agent 配置声明](agent-configuration.zh.md) 记录静态模型访问能力和可选 Settings，并生成原始字节
身份。release 清单和 Node 安装计划可以把该身份绑定到包，安装器在提交前核对包内声明。Agent 的
Settings 子集以声明的原始字节指纹进入本地设置存储和编辑器，模型访问能力在本地没有执行语义；
编译或安装声明都不会启用个人模型配置或授予执行权限。

## Hook

Hook 包附带一个自己管理 Agent 集成的程序；宿主从不自己做这份集成，只负责运行它的边界：

- 安装和运行是两个独立的授权。只有当请求带有 `hookExecutionAcknowledged` 时，单插件的安装、更新或导入之后才运行 `init`，卸载把包暂存走之前才运行 `deinit`。扩展包安装不为其成员运行任何东西。`initializeHook` 在显式请求时再次运行 `init`。
- 可执行文件在已安装的包内解析，每次启动前重新校验。它不经过 shell 运行，以 `~/.ora` 为工作目录，使用最小环境，不带任何设置值，限时 120 秒，输出有上限（采集 256 KiB，报告中保留 4 KiB）。
- 失败的命令从不改变安装状态。结果只为当前会话保存（`listHookLifecycleReports`），因为权威的事实是工具自己写入的 Agent 配置。

## 决策

- [带版本的双形态 manifest](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/manifest/0-versioned-dual-form-manifest.md)
- [静态 MCP 配置](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/category/mcp/0-static-mcp-configuration.md)
- [Hook 插件](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/category/hook/0-hook-plugin-pack.md)
- 其他插件种类：[`decisions/desktop/plugin/category/`](https://github.com/ora-space/specs/tree/main/decisions/desktop/plugin/category)
