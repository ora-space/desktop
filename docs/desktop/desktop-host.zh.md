# Desktop 宿主

[English](desktop-host.md) | 中文

`apps/desktop/src-tauri`（`ora-desktop`）是原生 Tauri 宿主。它启动共享的 `ora-backend::Backend`，向前端暴露命令，拥有原生窗口和对话框，并适配操作系统能力：交给文件管理器、在系统浏览器中打开 URL、插件 surface webview 和自更新。它不拥有领域持久化或 Agent 语义，这些都在 backend 和契约 crate 中。crate 内部细节见 [`apps/desktop/src-tauri/README.md`](../../apps/desktop/src-tauri/README.md)。

## 启动

1. `--help`/`-h` 和 `--version`/`-V` 在打开任何窗口、数据库或插件运行时之前打印并退出。其他参数都是正常的图形启动。
2. 数据根目录在设置了 `ORA_DATA_DIR` 时取它的值（相对值按 cwd 解析），否则取 Tauri 为标识符 `space.ora.desktop` 提供的 `app_data_dir`。
3. Desktop 在数据根目录上以不等待的方式获取独占的 `instance.lock`，并持有到进程退出；进程退出或崩溃时由操作系统释放。第二次启动会把焦点交给已有窗口并退出，不打开数据库，退出原因写到 stdout 或父控制台。假定没有其他存活进程的启动工作，例如工作流的启动扫尾，只在持有锁之后才运行。
4. 初始化日志（见[日志](../core/logging.zh.md)），然后启动 `ora-reaper` sidecar，再打开 Backend：SQLite 启动、Skill 存储协调、插件发现、Agent supervisor 和工作流运行恢复。

## 进程清理

`ora-reaper` 是用同一份源码构建的 sidecar。生产环境 `TokioProcessSpawner` 创建的每个进程都会通过私有的父进程存活管道登记到它。正常关闭时先释放进程所有者，再让 reaper 强制结束残留进程并等待确认；如果 Desktop 崩溃，管道关闭会触发同样的清理。进程树在 Unix 上是进程组，在 Windows 上是 Job Object。见 [`crates/process/README.md`](../../crates/process/README.md)。

## 命令层

命令实现按领域放在 `apps/desktop/src-tauri/src/commands/` 下。Desktop 在 `bindings.rs` 和 `bindings/` 里拥有构建期的 binding 目录：每个 unary binding 指明一个逻辑操作、它的 Rust handler 和显式的 Webview 权限；流操作指明领域启动 handler，复用共享的 `stream_contract`/`cancel_contract_stream`。`task export-contracts` 把这个目录与逻辑操作目录合并，生成私有的 TypeScript 映射、带类型的流路由、`app_commands.rs` 和权限文件；普通 Tauri 构建使用已提交的产物，从不运行 `xtask`。见[前端契约](../core/frontend-contracts.zh.md)。

访问权限按 Webview label 通过 `capabilities/` 授予：

| Label              | 可以调用                                                              |
| ------------------ | --------------------------------------------------------------------- |
| `main`             | 所有已注册的命令                                                      |
| `plugin-webview:*` | 只有 `plugin_webview_invoke`，见 [Surface](../plugins/surfaces.zh.md) |
| `remote-webview:*` | 什么都不能：没有任何 capability 针对这个 label，页面没有 Tauri IPC    |

`build.rs` 会把注册表和 `permissions/main-commands.toml` 对照检查，因此注册过的命令不可能绕开 ACL。

### 请求执行

一个共享的包装函数分配规范 request id，打开请求 span，调用领域方法，把 backend 错误投影成 `{ code, params, requestId }`，并最多记录一条完成事件。命令调用的是窄的 Backend 领域句柄（`backend.workspaces()`、`backend.plugins()`、`backend.sessions()`、`backend.workflow_runs()`、`backend.settings()` 等），而不是根上的转发方法；这些句柄自己负责并发、锁和提交后的工作，所以 Desktop 从不自己编排某个领域的状态转换。阻塞的领域工作在阻塞执行器上运行。

异步执行器（`run_async_backend`）接收的每个领域调用都是已经装箱的 future。Tauri 在主线程上的 WebView IPC 回调里构造每个异步命令的 future，这个栈本来就小，而且已经被占用了一部分；很深的领域 future（marketplace 安装链单态化后有几百 KB）会在运行时轮询它之前就把栈撑爆。装箱让命令 future 只有一个指针大小，Desktop 的测试还对插件传输命令设置了明确的 future 大小预算。

流的处理见[流与取消](../core/streams.zh.md)。

### 原生命令

除了生成的契约命令，Desktop 还注册了一些没有 SDK 操作的原生命令：

| 命令                                                                          | 用途                                                                                                       |
| ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `stream_contract`、`cancel_contract_stream`                                   | 共享的流传输                                                                                               |
| `get_worktree_root`、`set_worktree_root`                                      | 新任务 worktree 的创建位置                                                                                 |
| `resolve_task_cwd`、`resolve_workspace_cwd`                                   | 任务或 Workspace 当前的工作目录                                                                            |
| `open_location`                                                               | 在系统文件管理器中定位路径（`explorer /select,`、`open -R`），而不是用默认编辑器打开；目录以文件夹窗口打开 |
| `open_external_url`                                                           | 在系统浏览器中打开 `http(s)`/`mailto` URL；其他 scheme 返回 `InvalidRequest`                               |
| `write_workflow_export`、`read_workflow_import`                               | 工作流文件的导出和导入                                                                                     |
| `download_today_log`、`download_plugin_log`                                   | 把诊断日志复制到保存对话框选定的位置                                                                       |
| `surface_*`、`plugin_webview_invoke`                                          | 插件 surface，见 [Surface](../plugins/surfaces.zh.md)                                                      |
| `is_new_installation`                                                         | 数据库是否由本次启动新建，见[更新日志](../development/release-notes.zh.md#展示)                            |
| `get_desktop_update_status`、`check_desktop_update`、`install_desktop_update` | 见 [Desktop 更新](#desktop-更新)                                                                           |

## 持久化路径

| 数据                                 | 位置                                         |
| ------------------------------------ | -------------------------------------------- |
| SQLite 数据库（包括 `user_config`）  | `<数据根目录>/ora.sqlite3`                   |
| `0025` 迁移前 SQLite 升级备份        | `<数据根目录>/ora.sqlite3.pre-0025.bak`      |
| 日志                                 | `<数据根目录>/logs/ora.log`                  |
| 会话历史                             | `<数据根目录>/sessions/`                     |
| 定时运行产物                         | `<数据根目录>/scheduled-artifacts/<run id>/` |
| Skill 包                             | `<数据根目录>/atoms/skills/`                 |
| 实例锁                               | `<数据根目录>/instance.lock`                 |
| 插件、marketplace 源、插件数据和日志 | `~/.ora/plugins/`                            |
| 默认 worktree 根目录                 | `~/.ora/worktrees`                           |
| 已下载的更新包                       | `~/.ora/cache/desktop-updates/v2/`           |

worktree 根目录是非敏感配置，在 设置 → 数据与隐私 中选择，必须是一个已存在目录的绝对路径。它只是创建目标：新值只影响之后开始的任务创建，已有的 worktree 从不移动，也从不根据它重新拼路径，见[任务与 worktree](../workspaces/tasks-and-worktrees.zh.md)。

设置了 `ORA_DATA_DIR` 时，数据库里相对的项目和 Workspace 位置按数据根目录的父目录解析（在 `task run:desktop` 下就是仓库根目录），而不是进程 cwd，因为 `tauri dev` 是在 `apps/desktop/src-tauri` 里启动的。

## Desktop 更新

发布产物只覆盖 Windows（NSIS 安装包）和 Linux（AppImage 和 `deb`）。Ora Desktop 不再支持 macOS：v0.6.0 是最后一个 macOS 版本，更新清单里没有 macOS 条目，所以 macOS 上的安装停留在 v0.6.0。

发布构建在启动一分钟后检查一次已签名的静态清单 `https://github.com/ora-space/desktop/releases/latest/download/latest.json`，之后通过 `ora-scheduler` 任务每六小时检查一次（`0 0 */6 * * *`），并使用用户配置的代理。Tauri 更新器选择当前操作系统和架构并校验签名；Desktop 服务把通过校验的包存入按身份寻址的缓存，所以早先进程下载过的包可以复用（会先用配置的公钥重新校验），无需再次下载。开发构建保留这些命令供测试使用，但不注册任何网络任务。

- 已下载的包在后续检查失败时仍可安装；只有在没有任何可安装的包时才报告 `Failed`，安装失败会恢复到 `Ready`。`Failed` 写明失败的是检查还是安装（安装时带版本），外壳据此写标题。设置侧栏和更新日志页显示失败，更新日志页还显示原因、安装失败的提示，并可以重试。
- Linux 发布的是 AppImage，所以 `deb`/`rpm` 安装或裸可执行文件会在下载之前就报告 `ManualUpdate` 及原因，外壳会指向发布页而不是提供安装按钮。
- `Ready` 和 `ManualUpdate` 带有清单里的更新日志，见[更新日志](../development/release-notes.zh.md#更新之前)。
- 更新器公钥在 `apps/desktop/src-tauri/tauri.conf.json` 中；发布构建需要 GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY` 中对应的私钥。

状态变化以 `desktop-update-status-changed` 发出。生命周期、缓存布局和不变量见 [`src/update/README.md`](../../apps/desktop/src-tauri/src/update/README.md)。

## Cloud 连接

Desktop 可以登录一个自建的 Cloud 服务器，以企业账号身份调用它的 API；没有连接、未登录或服务器连不上时，本地的会话、Agent 和工作区照常可用。整件事由宿主里的一个协调器（`src/cloud_connection/`）拥有：服务器地址、登录事务、令牌和状态都在这里，主 webview 只能经五个命令（`cloudConnection.get` / `setServer` / `startLogin` / `cancelLogin` / `logout`）和 `cloud-connection-changed` 事件看到服务器地址、账号显示名和状态，插件 webview 两者都拿不到。

- **地址**：在 设置 → Cloud 连接 中填写，规范化为一个 origin（`https://host[:port]`，主机小写、去掉默认端口；只填主机名时补 `https://`）。带用户名、路径、查询串或 fragment 的拒绝；只有本机地址（`127.0.0.0/8`、`::1`、`localhost`）可以用 http。地址不是秘密，存在 `user_config.cloud_server`。改地址或清空会取消进行中的登录，删除旧令牌并请旧服务器吊销它；`setServer` 的回答说明旧服务器是否确认，没确认时面板和退出登录一样提示“服务器没有确认吊销”。
- **登录**（RFC 8252）：宿主在 `127.0.0.1` 的系统分配端口上监听 `/callback`，生成 `state` 和 PKCE verifier，用系统浏览器打开 `<服务器>/desktop/authorize`，带上回环回调地址、`state` 和 S256 challenge。服务器页面要求已有网页会话、由人点「允许」后，浏览器带着一次性授权码回到回环地址，回环页只写“已收到授权，请回到 Ora。”（这时还没换令牌）。在确认页点「取消」会带着 `error=access_denied` 回到回环地址，登录以“已取消”结束。路径或 `state` 不对的请求返回 404，不结束事务。宿主用授权码、verifier 和同一个回调地址调 `POST /auth/desktop/token` 换到令牌。事务 5 分钟超时，可以取消，同时只有一个；成功、失败、取消、超时和退出都会关闭监听。
- **令牌**：只在宿主内存和操作系统的安全存储里，不进 SQLite、webview、日志或 URL。条目属于“操作系统用户 + 服务器 origin”：Linux 上是 Secret Service 条目（服务名 `space.ora.desktop.cloud`，账号是 origin）；Windows 上是凭据管理器的普通凭据，名为 `<origin>.space.ora.desktop.cloud`，以 `CRED_PERSIST_LOCAL_MACHINE` 写入，只留在这台电脑，不随漫游配置文件同步。条目只按“操作系统用户 + origin”区分，所以开发版（`task run:desktop` 设了自己的 `ORA_DATA_DIR`，本地配置与正式包各自独立）和正式包共用它：在开发版里对同一 origin 登录或退出，会覆盖或删除正式包的条目。按应用标识判断的单实例插件阻止两者同时运行。安全存储不可用时状态为 `storageUnavailable` 并说明原因，Cloud 功能关闭，不降级为明文；刚换到的令牌写不进安全存储时，尽力请服务器吊销它。
- **请求与失效**：带令牌的请求只发往保存的 origin，不跟随任何重定向，证书用平台校验器验证，代理按环境变量（`NO_PROXY` 与本机地址直连，并关闭 reqwest 自己读取代理环境变量），不用只作用于 marketplace 源的代理设置。只有 `401` 表示会话结束：状态改为 `needsSignIn` 并删除令牌，但只在这次回答结束的正是它所问的会话时才删，迟到的旧令牌 `401` 不会删掉新登录写入的条目；网络错误、超时、重定向和 5xx 只标为 `unreachable`，保留令牌。启动时和主窗口重新获得焦点时（至少间隔 60 秒）调一次 `GET /api/v1/me` 检查。
- **登出**：先删除本机令牌，再调 `POST /auth/desktop/logout`；服务器连不上时仍然在本机退出，并提示服务器没有确认吊销。安全存储删不掉条目时仍然请服务器吊销，状态显示为 `storageUnavailable` 而不是未登录。

## 内置 sidecar

发布构建在可执行文件旁边打包 ripgrep 和 Deno，并在启动时解析它们；缺少其中一个时，Desktop 会在创建应用状态之前停止。debug 构建从 `PATH` 解析 `rg` 和 `deno`。sidecar 的下载和校验见 [Sidecar](../operations/sidecars.zh.md)。
