# Desktop 运行时

[English](desktop-runtime.md) | 中文

`apps/desktop/src-tauri` 是根 Cargo workspace 中承载持久化操作和 ACP 流的 Tauri 应用。命令按领域放在 `src/commands/`；`commands.rs` 只组合模块、宏及共享同步/异步请求执行机制。

## 共享 Backend 与命令

Desktop 在 `bindings.rs` 和分领域 `bindings/` 中声明宿主绑定。Unary 声明逻辑操作、Rust handler 和 Webview 权限；stream 声明领域启动 handler，共享 stream/cancel 和平台命令单独绑定。`task export-contracts` 联合逻辑 catalog 生成 TypeScript 映射、流解码与分派、命令注册及权限。主 Webview 和插件 Webview 授权分开；普通构建只消费生成物，`task check:contracts` 检查漂移。

一个可克隆 Backend 服务所有命令。共享 wrapper 分配 request ID、创建 span、执行业务并投影错误。Session load、prompt 和 app events 通过 Tauri Channel 传送有序 data/error/end 帧；call ID 控制单流取消，request ID 关联完整请求。注册先于领域启动，直到所有者释放才允许复用 ID；取消不会放弃尚未完成的资源创建，迟到资源立即清理。退出取消启动中和运行中的注册并拒绝新流。

异步 executor（`run_async_backend` 及其带 request ID 的变体）要求每个领域调用都以已装箱的 future（`Box::pin(..)`）传入，直接 await 领域 future 的手写命令同样装箱。Tauri 在主线程的 WebView2 IPC 回调内构造每个异步命令的 future，而该回调约 1 MB 的栈已被 handler 之下的 webview/Tauri 帧占用数百 KB。跨命令 await 持有的深层领域 future——仅市场安装链就单体化成约 650 KB 的状态机——会在异步 runtime 首次轮询之前溢出该栈；0.2.0 release 正是因此（WER `0xc00000fd`）在点击市场安装的瞬间崩溃。在调用点装箱使命令 future 保持指针大小，深层链只在首次轮询包装器的 runtime 线程上构造，而装箱参数类型使该约束由编译器强制。Desktop 测试以显式的 IPC future 尺寸预算锁定市场传输命令。

Agent、Skill、工作流定义、项目、任务、workspace、插件、workflow run、session 和运行时状态命令各使用窄领域句柄，不增加根 Backend 转发。共享 executor 管请求生命周期，领域负责阻塞工作、级联、状态提交和通知。Workspace 克隆共享锁和清理租约；插件所有者负责 reconcile，surface 仅取 gateway；workflow run 持有 run gate 并在终态提交后清理 session。Session 标题先持久化再更新 actor；app events 只暴露订阅能力。Git identity 使用无状态 Backend 导出。

前端将 `createTauriTransport()` 注入 `createContractsClient`，保持请求 DTO 不变。业务错误直接返回 `{ code, params, requestId }`；本地调用失败不伪造 request ID。Workspace 查询返回权威根路径和可选分支，事件流复用统一 framing、取消和完成机制。

开发者设置有四个 unary 命令：`get_developer_mode`、`set_developer_mode`、`get_runtime_log_level`、`set_runtime_log_level`，使用同一生命周期，不走 HTTP。Settings 只暴露偏好等窄能力，runtime manager 只获得受限日志偏好 store，不暴露 SQLite 或工作树根持久化。

Backend 对每个已安装 agent plugin 建立独立监督连接；插件拥有进程生命周期，session 共享 agent 连接并保留 ACP ID 和任务 cwd。连接失败不影响 shell 和其他 agent，目标不可用返回 `agent_runtime_unavailable`。UI plugin surface 使用隔离原生 webview，关闭最后一个 surface 后 30 秒停进程；生命周期先关闭 surface 再禁用、停止或卸载插件。插件存储经宿主方法访问。详见英文版链接的 Agent Runtime 和 Plugin Surfaces。

App Shell 等待 Ready 帧后挂载查询和 watcher。应用事件流是多订阅者尽力广播，不是持久化日志。平台命令包括工作树根读写、任务 cwd 解析和 `open_location`；explorer target 使用系统文件管理器定位，不启动默认编辑器。

## 更新与市场同步

Release 注册 Tauri updater 和 `ora-scheduler`：延迟首次检查，此后每六小时读取 GitHub Release 的 `latest.json`。Tauri 验证签名后将身份寻址产物写入 `~/.ora/cache/desktop-updates/v2/`，重启重新核对身份和签名后可复用。开发构建不调度网络更新。状态由 `get_desktop_update_status` 及事件提供，安装由 `install_desktop_update` 发起。公钥在 Tauri 配置，签名私钥来自 CI secret。

检查失败不清除已下载可安装包；安装失败恢复 Ready。Linux 自动更新只支持 AppImage，deb/rpm 或裸程序在下载前返回 ManualUpdate 并引导手动更新。

市场索引在启动十五秒后及每六小时刷新，按宿主本地时区调度，开发构建也启用。刷新按源独立进行：某个源失败时保留上一次索引中该源的条目，且不重扫它可能写了一半的检出，其余源照常刷新；只有被移除或禁用的源才真正丢失条目。失败原因属于哪个源就只影响哪个源：它自己的 Git 操作，或它勾选了代理但代理缺失/不可用；只有读取 Ora 自身状态（代理设置、源配置、命名空间绑定）失败才让整次刷新失败。源的 Git 操作走不走代理只由该源的代理开关决定：Ora 以命令级 Git 配置为源的 origin 和完整 URL 固定路由，优先级高于用户的 `http.proxy`、按 URL 配置的 `http.<url>.proxy` 和 `*_proxy` 环境变量；勾选代理的源总是使用 Ora 的代理，未勾选的源总是直连。失败随缓存记录并由 list/sync 返回，界面据此指明哪些源的列表陈旧，而不是把部分陈旧的目录当作刚刚同步；失败原因只保留 Git 自己的 `fatal:` / `error:` 诊断或配置问题，不含 Git 命令行和本地检出路径，完整错误只写日志。只有当至少一个源真正刷新成功时同步时间才前移，全部失败时保持原时间，从未成功过则显示「尚未同步」；没有任何启用源时不算失败，同步时间照常前移。失败等待下一周期，不自行重试。Backend 同时只接纳一次重建，重复请求返回缓存。自动同步事件使界面禁用 Sync，完成后使列表查询失效。

每个市场源独立选择 Direct HTTPS 或 S3 SigV4。后者接受 object key 或本 endpoint/bucket 的 path-style locator，拒绝外部 locator；默认 Direct HTTPS。S3 endpoint、bucket、region 和完整静态凭据一起配置，凭据只写不读，当前以 SQLite 明文保存，没有系统钥匙串、临时凭据或 provider chain。IPC 凭据不得进入日志，Debug 必须脱敏。配置错误返回可恢复的类型化错误，不回传非法值。两种方式共享代理和下载流程，安装前都必须校验 release manifest SHA-256。

## Skill 导入

四个 unary 命令 prepare/get/commit/cancel 复用 Backend 导入会话生命周期。前端仅传本地路径，最多 200 MiB 文件由 Rust 读取，不经 IPC 传字节。提交 catalog 前持久化包文件、journal 和目录 promote；Unix 目录 fsync 失败是硬错误，Windows 尽力执行，macOS 使用 fsync。准备、预览、冲突决定、后台提交和结果保留均由共享 Backend 负责。

## 持久化路径

Tauri identifier 为 `space.ora.desktop`：

- SQLite 及 `user_config`：`app_data_dir/ora.sqlite3`
- 日志：`app_data_dir/logs/ora.log`
- 默认新工作树根：`~/.ora/worktrees`
- Session 历史：`app_data_dir/sessions`
- Skill 包：`app_data_dir/atoms/skills`
- 插件、registry 及插件存储：`~/.ora/plugins`

首次创建数据目录和默认工作树根，SQLite 中已有选择优先。`ORA_DATA_DIR` 控制数据根；开发任务指向仓库 `.data`，数据库相对项目路径按数据目录父目录解析，不按 Tauri cwd。未设置时用 Tauri app data 目录。

用户可在 Data & privacy 修改工作树根，必须选择已存在的绝对目录。新值只影响后续创建，在途操作保持快照，已有工作树不移动。已有位置通过存储分支名及 `git worktree list --porcelain` 解析；任务和项目删除不直接修改 Git。

## 日志

Backend 打开前以明确的 `info` 初始化 `ora-logging` 并注册 Gitlancer 桥接。随后恢复 SQLite `log_level`，仅未设置时采用 `info`；存储读取失败中止启动。遗留 `ORA_LOG_LEVEL` 不再读取，没有启动覆盖。Settings 只拥有持久化偏好，`ora-runtime-settings` 负责过滤器、串行更新及保存失败后的回滚。每日轮转并保留三个文件，debug 同时写 stdout 和文件，release 只写文件，guard 保留至应用结束。

每个命令或流的完成事件与公开错误关联同一 request ID。取消在 DEBUG 完成，不投影 internal error。Git 清理排入持久化 worker，其独立 `git_cleanup` 事件不会改变原请求结果和错误链。

启动读取系统 IANA 时区并固定至进程结束；读取或解析失败记录警告并回退 UTC。系统时区变化重启后生效，文件仍按 UTC 日界轮转。Desktop 配置决策及不变的编译期上限见[运行时日志](runtime-logging.zh.md)。

## 验证

Tauri 共享根 Cargo.lock、依赖图和 target。`task test:frontend` 包含 Desktop transport；`test:crates`、`test:tauri`、`test:e2e` 分别覆盖共享 crate、Desktop 和 E2E。`task test` 运行全部，CI 分组执行。
