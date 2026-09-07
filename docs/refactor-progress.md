# 集成层重构：基线与验证记录

执行依据：本地 `drafts/refactor.md`。基线为 2026-09-06 的 `65ebdea`，目标是按功能所有权减少重复声明，保留数据布局、事务和运行时生命周期约定。本次按 Eric 指示跳过根决策检查。

## 进度

| 阶段            | 状态   | 交付                                              |
| --------------- | ------ | ------------------------------------------------- |
| 0：基线         | 已盘点 | 本文的登记点、command 与行为验证索引              |
| 1：contracts    | 已完成 | 显式响应模式、生成 client/DTO exports、确定性生成 |
| 2：Desktop      | 已完成 | 领域 binding、生成接线和注册 guard                |
| 3：Backend 试点 | 已完成 | settings 窄 interface、独立存储测试               |
| 4：Backend 迁移 | 已完成 | 领域用例与生命周期已迁移，旧根转发已删除          |
| 5：前端归属     | 已完成 | 翻译、共享查询与 typed 测试 transport 均已迁移    |
| 6：验收         | 已完成 | 门禁、两次实际增删演练与最终全量验收均通过        |

## 人工登记点

以下按信息来源计数，不把生成文件、测试、文档和真实业务实现算作重复登记。演练场景分别为在已有领域新增 unary 和新增 stream。

| 人工来源                                       | Unary           | Stream          | 计划归属                          |
| ---------------------------------------------- | --------------- | --------------- | --------------------------------- |
| `ora-contracts` DTO 及所属模块导出             | 是              | 是              | 保留：契约所有者                  |
| `xtask/src/frontend/namespaces/` operation     | 是              | 是              | 保留，增加显式响应模式            |
| `xtask/src/export_contracts.rs` 类型与模块映射 | 新 DTO 时       | 新 DTO 时       | 阶段 1 删除，使用生成 DTO exports |
| `packages/contracts/src/index.ts` DTO exports  | 新 family 时    | 新 family 时    | 阶段 1 生成                       |
| `packages/contracts/src/client.ts` 成员转发    | 是              | 是              | 阶段 1 生成                       |
| `xtask/src/frontend.rs` stream 名称分支        | 否              | 是              | 阶段 1 删除                       |
| `tauri-transport.ts` command map               | 是              | 否              | 阶段 2 由 Desktop binding 生成    |
| `tauri-transport.ts` stream union 与判断       | 否              | 两处            | 阶段 2 从声明生成                 |
| `app_commands.rs` 注册                         | 是              | 新共享机制时    | 阶段 2 由 Desktop binding 生成    |
| `permissions/main-commands.toml`               | 是              | 新共享机制时    | 阶段 2 从显式授权生成             |
| `commands.rs` 领域 stream 分发                 | 否              | 是              | 阶段 2 生成接线，领域提供启动实现 |
| `Backend` 根 façade                            | 经过 Backend 时 | 经过 Backend 时 | 阶段 3–4 由窄领域 interface 取代  |
| `test/mock-client.ts` 完整 client              | 是              | 是              | 阶段 5 改为按需测试 transport     |

最终演练应证明：人工仅维护契约、逻辑 operation、Desktop binding 和业务实现；增加领域允许增加少量显式组合项。生成物数量不作为失败指标。

## Desktop command 与授权基线

- 110 个 SDK operation：105 个 unary，5 个 stream。
- Stream：`loadSession`、`promptSession`、`watchAppEvents`、`watchWorkspace`、`watchProject`。
- 130 个已注册 command：105 个 SDK unary binding，加 25 个共享 transport 或 Desktop 原生命令。
- 共享 transport：`stream_contract`、`cancel_contract_stream`。
- workspace 原生命令：`get_worktree_root`、`set_worktree_root`、`resolve_task_cwd`、`resolve_workspace_cwd`。
- 本地交互：`open_location`、`open_external_url`、`write_workflow_export`、`download_today_log`。
- surface：`surface_capabilities`、`surface_list`、`surface_open`、`surface_close`、`surface_set_bounds`、`surface_set_visible`、`surface_popout`、`surface_dock`、`surface_reload`、`plugin_webview_invoke`、`surface_resolve_download`、`surface_discard_download`。
- 更新：`get_desktop_update_status`、`install_desktop_update`、`check_desktop_update`。

`default.json` 将 `allow-main-commands` 赋予 `main` Webview，并包含独立的 Tauri core/dialog 权限。`plugin-webviews.json` 仅向 `plugin-webview:*` 赋予 `allow-plugin-webview-invoke`，只允许 `plugin_webview_invoke`。重构后应逐项保持这两个 capability 的授权集合，不能从 handler 存在推断授权。

## 行为验证索引

此表记录已读取的代表性测试，不把源码存在等同于测试已通过，也不把局部证据等同于完整链路覆盖。迁移时补齐缺口并更新路径。

| 验证义务                          | 基线直接证据                                                                                                                            | 覆盖判断 / 后续义务                                                  |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| client 保留 DTO、返回值和调用选项 | `packages/contracts/tests/client.test.ts`：完整请求、unary options、stream 请求、所有 endpoint 成员                                     | 部分：补 stream AbortSignal 与所有 operation 的实际路由              |
| 公共错误与 requestId              | `apps/desktop/web/tauri-transport.test.ts`：`normalizes structured command errors`                                                      | 部分：错误解码器对 malformed/unknown 的完整验证需继续核对            |
| lazy stream、事件顺序、单次消费   | 同文件：`starts channel streams lazily and forwards ordered data until end`                                                             | 已有该路径的直接测试；取消竞争单独补证据                             |
| stream 队列限制                   | 同文件：`fails a channel stream when its bounded consumer queue overflows`                                                              | 已有该路径的直接测试                                                 |
| 取消和断开释放注册、只完成一次    | `stream_forwarding.rs`：`cancellation_completes_the_request_once_and_releases_the_registration`、两项 channel disconnect 测试           | 部分：预取消、创建中取消、重复 id、shutdown 需在阶段 2 逐项核对/补齐 |
| request 生命周期共享完成权        | `request_lifecycle.rs`：`cloned_lifecycles_share_the_id_and_one_completion_claim`、完成后 drop                                          | 已有直接测试；跨 Desktop 入口仍需集成检查                            |
| 共享 worktree 租约阻止独占删除    | `git_cleanup/keyed_locks.rs`：`exclusive_waits_for_shared_holders`                                                                      | 部分：底层锁证据不能替代 task/project 删除链路                       |
| cleanup 恢复和所有权检查          | `git_cleanup/tests.rs`：`expired_lease_is_reclaimed_and_cleaned`、`live_lease_is_not_reclaimed`、`ownership_loss_parks_without_removal` | 已有局部直接测试；领域迁移时保留                                     |
| 更换 worktree root 后删除旧 task  | `bootstrap.rs`：`deletes_existing_task_after_worktree_root_changes`                                                                     | 已有该升级场景的直接测试                                             |
| 存储重开和启动装配                | `bootstrap.rs`：`opens_storage_and_serves_shared_crud_apis`；Desktop `lib.rs` 重开测试                                                  | 部分：settings 独立持久化、失败及重新打开在阶段 3 验证               |
| workflow 并发完成、取消和恢复     | 尚未完成验证义务到代表性测试的映射                                                                                                      | 未验证：阶段 4 迁移前补齐，不能以纯 helper 测试代替                  |

## 检查记录

后续每阶段记录实际运行结果。跨范围实现阶段最终运行完整 `task test`，Desktop 变化另及早运行 `task test:tauri`。最终包含增删演练后的证据与剩余手写接点说明。

### 阶段 1（2026-09-06）

- `cargo test -p xtask`：17 项通过，包括未知 stream 增删、重复声明拒绝、生成清单漂移和手写文件冲突保护。
- `cargo clippy -p xtask --all-targets -- -D warnings`：通过。
- contracts lint 与 9 项测试通过；新增遍历全部 110 个 operation 的实际调用验证，覆盖 DTO、响应模式、返回值及选项。stream AbortSignal 传递有单独直接断言。
- `task export-contracts` 后执行 `task check:contracts`：通过。检查在临时目录生成，能够识别未跟踪的新增/失效产物，不修复工作树。
- `task test`：完整通过，包括 frontend、Rust workspace、Tauri 与 4 项 Desktop E2E。测试入口现在检查生成产物，而不是先覆盖产物再验证。
- 已删除手写 client 成员表、类型名到文件名映射和中央 stream 模式推断。`client-runtime.ts` 只保留映射类型及通用调用逻辑；DTO 与静态 factory 由生成器输出。

### 阶段 2a：命令归属（2026-09-06）

- `commands.rs` 从 1576 行降至 124 行，仅保留请求执行、命令宏和领域组合；具体实现位于 `commands/`。
- task/settings 原有入口已迁入同一领域目录；删除两套重复请求执行逻辑，文件读取也使用可注入 context 的通用阻塞执行方法。
- workspace listing/diff/location、Effect status 和 workflow export 按职责归属；旧路径引用同步更新。command 名称、DTO 与 capability 集合未改变。
- `task test:tauri` 通过：包含 Clippy、53 项 Desktop 测试；最终阶段 2 仍需验证生成 binding、stream 竞争及全量测试。

### 阶段 2b：声明、分派与取消所有权（2026-09-06）

- Desktop 的 `bindings/` 按领域拥有 handler、响应接线及显式授权；xtask 关联逻辑 catalog，生成 transport map、stream 分类、类型化请求分派、Rust registry 与权限清单。公共 manifest 不含 Tauri 路径或授权。
- 与 `65ebdea` 逐项比对：主 Webview 的 130 项实际授权、plugin Webview 的唯一授权均不变；两个 capability JSON 逐字不变。生成过程去除了原权限列表中重复的一项 `rename_workflow_run`，不改变授权集合。
- 注册 guard 在领域创建前取得 id；取消只发信号，释放资源后才允许复用 id，避免旧任务清掉新注册。退出时统一取消并拒绝新注册；创建中取消等待创建安全结束后释放资源。
- 新增 catalog 完整性、错误模式、重复 handler、typed stream 增删和实际 plugin grant 校验；Rust 覆盖预取消、创建中取消、创建失败的 requestId、重复 id、shutdown。Frontend 覆盖创建中发出取消、终止错误及 requestId、未知 operation、预取消不启动 IPC；保留单次消费、顺序、end 与溢出验证。
- 验证：xtask Clippy 和 20 项测试通过；Tauri Clippy 和 58 项测试通过；Desktop frontend 41 项测试通过。最终 `task test` 全量通过，包含 4 项 E2E。

### 阶段 3：settings 试点（2026-09-06）

- 原私有 `UserConfigApi` 成为明确导出的 `Settings`，仅开放设置用例。构造、SQLite、原始配置键和 worktree root 持久化保持内部可见；没有新增 trait 或把 repository 暴露给 Desktop。
- 删除根 `Backend` 的 9 个设置入口，以一个 `settings()` 入口替代；proxy probe 和日志存储 capability 由 settings 拥有。请求执行和 updater 不再持有整个 Backend，runtime logging 仍只得到受限的 preferred-level store。
- 去掉启动中同一配置 module 的重复构造；通用的 Backend repository 执行机制独立于 bootstrap，避免领域 module 反向依赖启动装配。未改数据库或目录布局。
- 将完整 runtime CRUD 测试里的设置断言迁到 `settings/tests.rs`，补上重新打开、真实 SQLite 写失败及读失败测试；3 项通过。原 Desktop 重开/启动覆盖保留，且已迁移到窄 interface。
- 试点净收益：不是多套一层转发，而是移除根转发和不必要的 runtime 所有权，设置测试只需 SQLite。可以据此扩大阶段 4。
- 额外尝试的 `cargo clippy -p ora-backend --all-targets -- -D warnings` 被现有测试大量使用 `unwrap`/`expect` 阻挡（仓库标准 lint 不包含这些测试目标）；不顺带改写无关测试，最终按 `task test` 的标准门禁验收。
- 最终 `task test` 全量通过：含 208 项 Backend 测试、58 项 Tauri 测试和 4 项 E2E；标准 Rust/Frontend lint 均通过。

### 阶段 4a：普通领域用例（2026-09-06）

- 30 个 agent 定义、Skill 和 workflow 定义操作不再占用根 `Backend` 方法，调用者使用 `agents()` / `skills()` / `workflows()`。只导出所属用例，构造与字段仍隐藏，错误投影收进领域 module。
- Desktop 命令捕获对应领域 handle；surface 的自动下载和用户确认导入都使用 Skill interface。原导入流程、事务和 Effect 唤醒归属不变，未改动生成 binding 或任何 operation/DTO。
- 新增仅用 SQLite 的 agent CRUD/错误投影、workflow definition/draft 重开测试；既有 Skill 存储互斥和真实导入/Effect E2E 继续承担原验证义务。
- 此处只是阶段 4 的第一组。旧 command 宏分支暂时只服务尚未迁移的其他领域，最终阶段 4 收拢时删除；不把拆出这些普通用例等同于完成生命周期协调迁移。
- 验证：Backend 209 项通过、1 项原有测试保持 ignored；`task lint:crates`、`task test:tauri`（58 项）、`task test:e2e`（4 项）和 `task check:contracts` 通过。阶段 4 完成时再运行全量检查。

### 阶段 4b：project/task 聚合用例（2026-09-06）

- 再移除 12 个根 operation 转发，以 `projects()` / `tasks()` 交付完整用例。SQLite 级联、活跃后代检查、历史清理、阻塞线程派发和提交后的 Git cleanup 唤醒由所属领域拥有，Desktop 不再了解这些步骤。
- 新的 crate-private `TaskSetup` 显式注入原有 provisioning gates 与 cleanup handle，未新建锁或改变共享关系；根 Backend 不再保留只为转发而持有的 cleanup handle，删除无调用者的公开 repository pool 入口。
- project/task 两个 Desktop 删除命令原先绕过通用 lifecycle，现使用相同的 async executor，成功及失败都有相关联的完成记录。
- 旧 worktree-root 变更后删除测试移至 `task/lifecycle_tests.rs`，两项生命周期测试均在 scoped TRACE 下执行。新增真实 SQLite/Git 验证：Running session 同时阻止 task/project 删除且保留完整对象；停止后 task 删除连带隐藏 session，已有 worktree use lease 保证物理目录不被提前移除。
- workspace 查询、Git 操作及配置归属尚待下一组迁移；本组不改变磁盘布局和 Git cleanup 的恢复/退出语义。
- 验证：Backend 210 项通过、1 项原有测试 ignored；标准 Rust lint、58 项 Tauri、4 项 E2E 与生成漂移检查通过。

### 阶段 4c：workspace（2026-09-06）

- `WorkspaceDiffApi` 扩展为所属 module 的 `WorkspaceApi`，整合查询、live cwd、worktree-root 配置和 Git review；移除 10 个根入口，其中无调用者的 persisted-root 原始行查询不再公开。
- Desktop workspace/files 命令只注入 cloneable workspace handle；泛用文件浏览、搜索和 watcher 创建仍在 Desktop/`ora-fs`，没有迁移到 Backend。
- handle clone 共享原来的 root `RwLock`、SQLite pool 和 cleanup use leases，不新建锁/worker。`workspace.rs` 生产部分约 330 行；既有 main-checkout、task worktree 和 diff/commit/push 测试随 module 移动。
- 验证：Backend 210 项通过、1 项原有测试 ignored；标准 Rust lint、58 项 Tauri、4 项 E2E 和生成漂移检查通过。

### 阶段 4d：插件操作与 runtime 协调（2026-09-06）

- 20 个根入口（含 gateway 与下载进度变体）迁至 `Plugins`。安装、导入、更新、删除和扫描后的 agent-set 同步仍由同一 `AgentRuntimeManager` 完成，调用者只执行一个领域用例。
- 新协调代码放在约 210 行的 `plugin/operations.rs`。原大型 `PluginApi` 保持 crate-private，继续作为 runtime/Effect/configuration/gateway 共用的 host implementation；没有复制 lifecycle、锁、registry 或 generation 规则。
- 安装冲突与 README 测试迁至该 module，并通过公开 `Plugins` interface 执行；Tavily/configuration 场景从 bootstrap 随职责移动。需要外部发布产物的一项测试仍 ignored，其他依赖 `.tmp` 产物或 `ORA_E2E_PLUGIN_DATA` 的场景保留原条件，不将缺少 fixture 时的早退当成完整集成证据。
- 验证：Backend 210 项通过、1 项 ignored；标准 Rust lint、58 项 Tauri、4 项 E2E 与生成漂移检查通过。本次未配置 live plugin-home fixture。

### 阶段 4e：workflow-run 生命周期（2026-09-06）

- 12 个根 operation 迁到 `WorkflowRuns`，连同手工完成的 claim/prepare/revalidate/commit 和取消后的 session 清理一起迁移。Desktop 的 cancel/complete 也进入统一 async lifecycle，成功和失败均有 requestId 关联的完成记录。
- `WorkflowRunSetup` 注入原来的 engine、runtime、run locks 和完成中集合；自动回调、手工操作与尚待迁移的 session prompt 仍共享同一实例，没有新建 supervisor 或锁。
- boot sweep 和 baseline pruning 迁到 `workflow/run/recovery.rs`，启动调用顺序与 best-effort 语义不变。新的 operations 生产文件小于 500 行。
- 6 项公开 interface 测试使用生产 engine/runtime 与真实 SQLite：并发完成恰好一次、取消/完成竞争、history 读取失败后释放 claim 重试、session 清理失败不撤销取消、重开保留 awaiting node 并清理 orphan baseline、重开失败化中断 turn 并恢复 stalled run。没有外部 agent 进程的场景不被当作活跃 actor 取消证据，后续 session 迁移仍须覆盖。
- 测试共用已有真实数据库 fixture；内部 turn-policy 测试也改为 scoped TRACE 执行，避免共享日志 callsite 污染。
- 验证：Backend 216 项通过、1 项 ignored；标准 Rust lint、58 项 Tauri、4 项 E2E 与生成漂移检查通过。

### 阶段 4f：session 与 workflow prompt 协调（2026-09-06）

- 13 个 session operation 和 app-event 订阅转发从根 Backend 移除。`Sessions` 直接拥有原查询/改名 handlers，连同 unpublished-session 过滤、title actor adoption 和提交后的通知一起封装，不再叠加私有 `SessionApi` 转发。
- workflow-run 创建唯一的完成中集合；通过 crate-private `WorkflowSessionTurns` 向 Sessions 提供 prompt 准入与失败/stream-drop 清理能力。根 Backend 不再持有 run locks 或完成中集合，Desktop 只捕获 session handle。
- 缺少 agent 时的历史回放测试改为通过公开 Sessions 执行；新增真实 SQLite 改名成功/失败通知测试，以及 prompt 启动失败在返回前恢复 awaiting node 的测试。
- E2E fake ACP 增加显式 held prompt：收到首帧后一直等待真实 ACP cancel，不靠延时制造竞争。新增真实 actor/进程链路验证 stream drop 后 session 可复用、活跃 session 删除后记录与历史清理、workflow 取消后 actor 停止，以及人类 follow-up stream drop 后恢复 awaiting 并可手工完成。
- 验证：Backend 219 项通过、1 项 ignored；标准 Rust lint、58 项 Tauri、7 项 E2E 与生成漂移检查通过。runtime status、Effect status 和无状态 identity 是阶段 4 余下收口项。

### 阶段 4g：根组合收口（2026-09-06）

- `AgentRuntime` 只公开 readiness/model discovery；`Effects` 只公开持久化 target status。Git identity 改用明确导出的无状态函数，Desktop 不再为它捕获 Backend。
- 根 Backend 现在只保留路径/启动、领域 handle 和访问器，生产部分约 340 行。删除最后 4 个 operation 及两条旧 command 宏分支，不保留兼容转发；原 pool、锁、supervisor 和 worker 的实例关系、恢复顺序、退出语义与磁盘布局不变。
- Skill package 更新保留附加文件的测试随职责移到 Skill module；根启动测试保留装配与 plugin Skill 投影验证，并改为 scoped TRACE。
- 新增 Effects 无 runtime 的真实 SQLite 缺失/读失败区分测试；已有 Effect E2E 现在同时经公开 interface 查询 materialization 的 target status，两种 selector 指向同一 target。
- 最终 `task test` 全量通过：Frontend lint/clean-stderr 测试、Rust workspace lint/测试、220 项 Backend 测试（另 1 项原有 ignored）、58 项 Tauri 和 7 项 E2E。生成漂移检查也通过。

### 阶段 5a：feature 翻译资源（2026-09-06）

- `i18n-instance.ts` 从 3437 行降到约 60 行，只保留唯一实例和同步初始化/locale 存储。14 组纯数据资源由 `i18n/resources.ts` 显式组合；feature 不动态注册，也不为加载翻译引入 React implementation。
- workflow editor 拥有历史 `settings.workflow.*` 文案；Settings 的 plugins/Skills/Roles 分别拥有自己的文案和相关 contract errors。真正共享的 shell/transport copy 保留独立公共资源。
- 对比 `0e1581f` 的完整字典：所有原有中文 1464 项和英文 1474 项的 key/value 均不变。唯一新增是未被使用的 `chat.selectedFileLines` 中文文案；英文多出的其他原始键来自 9 组合法 plural variants，不做机械复制或删除。
- 组合校验按逻辑键比较两种语言，检查缺失复数形式和跨 feature 的重复所有权；保留具体 raw keys、fallback、`ora.locale` 和同步初始化语义。
- 14 项 i18n 定向测试通过；新增语言切换、blocked-storage、独立纯资源加载、重复键与 plural 缺失验证。app-shell lint 和完整 clean-stderr 测试通过：137 个文件、1267 项测试；规则见 `docs/frontend-ownership.md`。

### 阶段 5b：查询、失效规则与订阅归属（2026-09-06）

- 删除中央 `query-keys.ts`，由 12 个数据领域直接拥有 factory；对照 `96f00e9` 执行全部 37 个 factory 的普通、null 和空参数比较，实际 tuple 均保持一致。
- `state/data/` 收拢定义/draft/version、真实与 memory workflow-run 的查询与缓存更新。共享 runtime context/provider 归 shell 装配，终态判断归 `@ora/workflow-runtime`，数据模块不再依赖 feature 私有实现。
- workspace/session module 拥有 authoritative response adoption、项目/任务级联缓存清理、列表刷新及 tree placement；UI hook 保留选中项、草稿/composer 和活动状态协调。rename 不主动 refetch，standalone/deferred delete 仍区别处理。
- 插件、agent availability/models、Skills 的刷新范围通过显式 `plugin-lifecycle` 协调；启动、停止、mutation settled 与外部事件保留各自不同的失效集合和 await 语义。configuration 响应同时更新 detail/list 的规则也由插件数据拥有。
- Files scope/access、批量失效与 watcher/reconnect 迁到数据归属；Explorer/Search 仍显式组合，不增加通用 view 容器。新增测试证明切换 workspace 和 unmount 会结束对应 stream，缓存保留各自 listing。
- 原接口测试随实现移动；用真实 QueryClient/Observer 验证项目 main/task session 清理、分工作区 diff/file 隔离、rename 两端、rescan、插件失效矩阵、session gap refetch，以及真实与 mock run key/prefix 分离。
- `task test` 全量通过：app-shell 141 个文件、1277 项 clean-stderr 测试，workflow-runtime 60 项，以及 Rust workspace、58 项 Tauri 和 7 项 E2E。原 Backend 外部发布产物测试仍有 1 项 ignored。

### 阶段 5c-1：真实 client 的测试 transport seam（2026-09-06）

- 新增按 catalog 推导 request/response/mode 的 `TestHandlers`，`createTestClient` 直接使用生成的生产 client；没有新增手写 namespace 镜像。未注册 unary、未知 operation、错误 mode 和 prototype 继承 handler 明确失败；stream 在消费时才解析 handler。
- 6 项测试验证 DTO/bigint/options 原样传递、惰性 stream 与 iterator cleanup、未配置调用失败，以及 request/response mode 的编译期约束。Files scope-switch 回归改为只注册实际依赖的 4 个 operation，继续覆盖生产请求构造链路。
- app-shell lint 和完整 clean-stderr 测试通过：142 个文件、1283 项测试。本提交只建立并试用 seam；剩余内存 adapter 和测试显式组合迁移仍待完成，旧 mock client 尚未删除。

### 阶段 5c-2：领域内存 adapter（2026-09-06）

- 原 1247 行完整 mock client 的 CRUD、配置冲突、workflow 版本/run 投影、默认 model fixture 等行为迁入 `test/memory/` 对应领域。state 字段与构造也随归属移动；所有 handler 根据生成 catalog 检查 request、response 和 stream 模式。
- 原 mock client 现在仅是约 100 行的临时组合器，返回真实生成 client；既有全量测试也经过生产请求构造。原来只会抛 `not implemented` 的占位 handler 不再注册，由 transport 报告未配置。
- workspace 查询、Agent/Skill mutation、installed/available plugin 查询测试已分别显式选择需要的领域 adapter。原 workspace 错误测试的 `unknown` 双重强转和 client monkey patch 改为类型化的单 operation handler。
- 新增经生产 client 的 workspace adapter CRUD/隔离测试，确认选择 workspace 不会隐式配置 session 或 app-event stream。app-shell lint 和完整 clean-stderr 测试通过：143 个文件、1285 项测试。
- 阶段 5c 仍未完成：其余测试的全领域组合器调用及 client override 需要迁移，最终删除 `test/mock-client.ts`，不能把当前过渡形态当作完成。

### 阶段 5c-3：删除全领域测试组合器（2026-09-06）

- 用既有测试的实际 operation 调用（只记录名称，不记录 DTO）核对剩余 45 个文件的依赖，逐文件改为明确的数据状态与 adapter 组合。临时审计代码已随旧文件移除，不纳入生产或长期测试机制。
- 13 个文件不需要任何默认 operation，直接从 `createTestClient({})` 开始；不保留无意义的空状态 factory。其余 fixture 只初始化本场景需要的领域记录，类型由本地构造推导，不再共享完整 `MockClientState`。
- 删除 `test/mock-client.ts` 及其全部调用/导入。删除的是已迁移的临时组合器，无持久化数据变化；历史提交仍可恢复。
- 渲染测试直接引入本文件需要的 i18n 实例模块，避免依赖 worker 内其他文件的先后顺序。app-shell lint 和 143 个文件、1285 项 clean-stderr 测试通过。
- 阶段 5c 的剩余工作是把旧的 client 直接替换改到 typed handler seam，随后运行全量验收；不将“删除了中央文件”单独当作阶段完成。

### 阶段 5c-4：测试注入收口（2026-09-06）

- 自定义响应、失败注入、scripted chat stream 和 client 副本全部改为 typed operation handler；不再直接替换生成方法，也不再把不完整对象强转为 `ContractsClient`。只观察、不替换实现的 public-client spy 保留，用于验证 UI 调用。
- 迁移包括 84 处直接 override、22 处嵌套 client 副本及 callback/spy 注入。Surface 下载测试也移除旧的强转 stub，并补齐真实契约要求的时间和进度字段。
- 生产 client 继续构造请求并传递 options；handler mock 的断言包含第二个 options 参数，不为了兼容旧断言改变 transport 行为。
- `task test` 全量通过：app-shell 143 个文件、1285 项 clean-stderr 测试，Rust workspace lint/测试、58 项 Tauri 和 7 项 E2E。原 Backend 外部发布产物测试仍有 1 项 ignored。阶段 5 完成，阶段 6 的门禁和增删演练仍待实施。

### 阶段 6a：前端 interface 检查（2026-09-06）

- 11 个 feature 的本地 `interface.json` 按导出名称声明 public interface 和用途；未声明 module/符号默认私有。没有新增 runtime registry、全量 barrel 或跨目录通配豁免。
- 共享 agent catalog、review 尺寸规则、composer quote action 移到 state 归属；编辑和执行视图共用的 zoom 约束移到 workflow-node-chrome。调用者和测试/mock 路径同步迁移，不保留旧路径转发。
- `check:features` 使用 TypeScript resolver 和真实导出符号，覆盖静态/type/alias/re-export、动态/CommonJS 和 mock 访问；state 禁止依赖 feature，纯翻译只向单一 i18n 组合开放。门禁接入 `lint:frontend`，继而进入 `task test`。
- 6 项门禁正反例测试、tooling 检查和 app-shell lint 通过；143 个文件、1285 项 clean-stderr 测试通过。Rust 规模门禁、临时工作树增删演练和最终全量验收仍待完成。

### 阶段 6b：Rust 规模与存量债务门禁（2026-09-06）

- `ora-utils::rust_source` 按语法 span 排除测试，保留测试声明之后的生产代码；支持复合 cfg、cfg_attr、inline/外部 module、显式路径和 Unicode/BOM/shebang。重依赖仅在 `rust-source` feature 开启，不引入 Ora 领域或其他 Ora crate 依赖。
- xtask 结合 Cargo targets 与全部已跟踪/非忽略的新 Rust 文件追踪生产和测试引用；integration target、测试专用文件及后代被排除，生产/测试共享文件仍计入，未引用文件保守计入。路径判断复用 ora-utils containment/normalization，不改动产品磁盘布局。
- 按“不含测试与空行、包含注释”统计，初始 671 个文件中 80 个为测试专用；30 个超过 500 行目标，7 个超过 800 行硬门槛。仅这 7 个已有文件记录 owning crate、精确上限和具体拆分计划；增长、未登记超限、缩减后未下调基线及过时例外都会失败。
- `check:rust-size` 接入 Rust lint；12 项解析测试、5 项引用图测试、4 项基线策略测试及定向 Clippy 通过。长期规则和存量责任见 `docs/architecture-checks.md`。
- 集成门禁后的 `task test` 全量通过：tooling、frontend lint/测试（app-shell 1285 项）、Rust workspace、58 项 Tauri 和 7 项 E2E；原 Backend 外部发布产物测试仍有 1 项 ignored。阶段 6 的临时增删演练仍待完成。

### 阶段 6 验收发现：Desktop 类型检查缺口（2026-09-06）

- unary 演练额外运行 Desktop TypeScript 检查时，发现阶段 2 新增的取消测试使用了 ES2024 的 `Promise.withResolvers`，不符合仓库 ES2023 lib。既有 Desktop lint 只运行 ESLint，因此此前完整 `task test` 没有覆盖这个编译入口。
- 测试改为显式可完成的 ES2023 Promise，保持“创建中取消”的异步边界不变；Desktop lint 加入 app 和 node 两份 tsconfig 的类型检查，不提高产品运行目标，也不依赖临时的 lib 覆盖参数。
- 修正后的 Desktop lint 和 41 项 transport/platform 测试通过。演练从包含此修正的干净基线重新执行；最终全量验收将包含新增类型门禁。

### 阶段 6c：临时增删演练与操作指南（2026-09-06）

- 基于 `1723a13` 的独立工作树实际增加并删除 Settings unary 和 Files stream。分别只修改 5/4 个所属实现与声明文件；四类手写事实不变，6/5 个接线产物全部生成，测试另外计数。
- 两次新增的重复生成均一致，真实 client/DTO/options 测试、Desktop 类型/lint 和 58 项 Tauri 测试均通过；unary 另有真实 SQLite interface 测试，stream 另有冷消费和共享 cancel 路由测试。没有把 mock transport 验证夸大为原生端到端覆盖。
- 删除手写部分后，检查分别检测到 6/5 个残留产物并失败；重新生成后均通过，工作树状态为空，恢复到 105 unary、5 stream、130 项主 Webview 授权。根 Backend 与通用生命周期实现未修改，其他 Webview 未扩大权限。
- 临时工作树与 probe 功能已清理，未删除用户数据。详细计数与复现证据见 `docs/refactor-acceptance.md`；feature 所有权、契约、授权、缓存、订阅、测试和文档清单见 `docs/feature-change-guide.md`。

### 最终验收（2026-09-06）

- 最终主工作树 `task test` 全量通过，包含新接入的 Desktop app/node 类型检查、feature/Rust 规模门禁、生成检查、tooling、全部前端和 Rust workspace lint/测试、58 项 Tauri 和 7 项 E2E。app-shell 为 143 个文件、1285 项 clean-stderr 测试；Backend 为 220 项通过、1 项原有外部发布产物测试 ignored。
- 搜索确认旧 query-key/mock-client 入口及 probe operation 未留在产品源码；根 Backend 仅保留启动与领域 handle。方案阶段 0–6 完成，不保留临时兼容链路；7 个既有超限 Rust 模块按方案记录为后续受控债务，不宣称整个仓库已无架构债务。
- 本次仍跳过根决策检查，未修改 specs/memories、用户数据布局或远端仓库。
