# 进程宿主 Scope 与 Run 意图日志

[English](storage.md) | 中文

Linux `ora_process_runtime::HostState` 已负责持久化 Scope／Run 意图与一次性
[独立 guardian 启动](../guardian.zh.md)，由[宿主 app](service.zh.md) 组合自动协调。
它是已批准[guardian 启动决策](../../../specs/decisions/node/process/recovery/20260917-rootless-guardian-bootstrap-and-reconnect.md)
的部分实现，无需 root、helper、cgroup 委派或服务安装。现有 Backend Git／插件入口及业务数据库政策不变。

## 显式定位与所有权

调用方提供绝对路径、专用的 `state_dir`；`HostState` 不读取 HOME，也不使用业务 cwd。
部署将 host 目录与 Node 数据目录分别注入；[独立 Node](../../node/runtime.zh.md) 显式接收该 host 路径，用于受管 Git。

- `HostState::create(&state_dir)` 要求目标目录不存在，已有空目录也拒绝。只创建
  `host.lock`、`host.sqlite`、SQLite 辅助文件及 `scopes/`。
- `HostState::recover(&state_dir)` 要求原稳定锁、数据库和 scopes 目录存在。缺文件、未知根目录条目、
  畸形身份及不兼容日志均失败，不重置状态；恢复失败后不会自动改走创建入口。
- 目录仅所有者可访问；文件必须是仅所有者可访问、只有一个硬链接的普通 inode。复用
  `ora-utils::path` 拒绝符号链接及组／其他用户可写的祖先目录，不修改已有权限。
  root 和指定 UID 仍属于可信范围，不隔离恶意同 UID 程序。
- 首批文件系统允许 ext 家族、XFS 和 Btrfs，拒绝网络、内存、overlay 及未知文件系统。
  分类本身不证明挂载选项、存储硬件或断电行为；本地测试目前只覆盖 ext 家族存储。
- 完整的规范 Scope ID 与 `control.sock` 后缀必须满足 Linux 路径型 socket 长度上限。
  超长路径拒绝，不截短、不换目录。组可写的 checkout 和 `/tmp` 不属于支持的状态父目录。

创建中断可能留下不完整的专用目录；恢复会保留并拒绝该目录，不自动修复；仅支持下述精确 v1 至 v5 日志的迁移。
持有期间不得删除锁文件、替换目录，或删除记录来绕过失败。

## 持久事实不等于启动权限

宿主持有原 `host.lock`，直到 SQLite 连接关闭。恢复先非阻塞获取原锁，再只读检查日志兼容性，
最后才提交新的宿主实例。锁竞争返回错误，不授权替换锁或 endpoint。

版本 6 日志使用 application ID `0x4f524148` 和 `user_version=6`，检查精确 schema、完整性及
持久身份。记录正数宿主代次及宿主实例 ID，以及各 Scope 的原 guardian 实例、创建时宿主绑定和
`intent_recorded` 阶段。代次溢出拒绝；恢复不改写意图中的创建者。
已有 Scope 目录必须具有规范 ID、私有目录元数据和匹配的宿主意图。本切片不检查或管理其内部
journal 与 endpoint。

`record_scope_intent(scope)` 提交新的原始意图，或原样返回已有记录；`scope_intent(scope)` 查询
该责任。查询不存在不代表可以重建 guardian。此登记调用不创建 Scope 目录、guardian journal、启动票据、
进程、凭据或 Ready 事实。没有意图的已有 Scope 路径会阻止登记，原文件保留。

`start_guardian(scope, executable)` 要求已有意图及显式传入的可信可执行文件。
它先提交 `launch_unknown` 记录，再创建私有 Scope 目录、获取其锁并 exec guardian。
此后的错误或取消均消耗这次尝试；即使证明 exec 失败，也不能再次启动。
`guardian_access(scope)` 在宿主恢复后取回原发现材料，不启动进程，也不转移控制权。
Ready 由 `ora-process-client` 另行查询。

恢复接受精确的 v1 仅意图 schema，在同一事务中添加启动表并推进宿主身份；原意图、路径与锁 inode 不变。
旧版本不能启动 guardian，因此没有需要推断或回填的启动记录。精确 v2 schema 仅在没有 Scope 目录时迁移：此时移除废弃凭据列，但保留全部已消耗的启动尝试。
已有旧 Scope 时，在写入身份或移除凭据前就拒绝；旧 guardian 可能仍需要原令牌协议。
应保留兼容旧 host 管理这些 Scope，或为新任务另选专用状态目录，不得删 Scope 来绕过检查。
这不是对历史 SQLite 空闲页及备份中的令牌进行安全擦除。未知 schema 拒绝；旧二进制拒绝 v6，不会重置它。宿主恢复不写 guardian.sqlite。

日志独立启用并验证 WAL＋`synchronous=FULL`，要求实际链接的 SQLite 主线版本包含 WAL-reset 修复
（不低于 3.51.3）。新事实提交事务后才返回，并同步所在目录条目。因此，提交后的文件系统失败
可能使调用报错但记录已存在：应查询原身份，不能从错误推导“没有接受”。SQLite 的耐久语义见
[WAL](https://sqlite.org/wal.html) 和 [synchronous](https://sqlite.org/pragma.html#pragma_synchronous)
文档；物理断电耐久仍未验证。

## Run 意图与恢复发现

向 guardian 发送 Start 前，先调用 record_run_intent(intent)，保存协议所有的 HostRunIntent。
要求 Scope 意图已存在；持久保存不变的 ScopeId、RunId、精确 RunSpec 和显式宿主断连策略。
登记前检查完整 guardian 请求满足当前帧上限。环境值原样保存在私有数据库中；
非 UTF-8 的路径、参数和环境值可无损往返。

相同 RunId、相同意图返回原记录，修改所属 Scope 或参数则冲突。提交并同步后才返回。
登记本身不创建 Scope 目录、不启动 guardian、不派发 Start，也不承诺自动后台执行。
host 意图、guardian 接受、进程事实及 Node 业务成功是不同的事。

run_intent(run) 查询单条责任，run_intents() 按稳定 RunId 顺序枚举全部 host 记录，
包括已终结或尚未派发的尝试。当前进程内枚举的内存用量随保留日志增长，尚无退休或分页政策。
恢复后可根据 guardian_access(intent.scope) 找回原 guardian，绑定新 host 实例，
再通过 GuardianRuns 查询原 Run；确认回复丢失不要求新建 ID。

v4 之前的 host 格式在推进宿主绑定的同一事务内添加空 Run 表。v3 可能已经有存活的
guardian／Run；迁移不读取 guardian.sqlite，不虚构缺失的 host 意图。
host 查询为空不能证明旧 Run 未执行。未知格式、索引与负载不一致、孤立 Run 引用或负载损坏，
均在推进宿主身份前拒绝。

## 持久停止和关闭意图

`request_run_stop(run)` 登记强停责任；`request_scope_close(scope)` 永久封闭 host 的接纳入口。
两者均先提交再回复，重启后仍然生效。已记录 Run 的重复 Start 仍返回原意图，但关闭中的 Scope
拒绝新 Run 和新 guardian 启动。关闭隐含停止其全部 Run，不改写原始参数。未知身份拒绝。
这些记录不证明已发送信号或完成清理。

`run_stop_requested`、`scope_close_requested` 和 `scope_intents` 向 host 协调器暴露持久责任。
旧格式在同一事务内增加空控制表，保留 v4 Run 意图；孤立控制引用在推进宿主权威前阻断恢复。

## 验证与剩余工作

`HostCoordinator` 组合日志与 guardian client，由所有者持续驱动 `tick()`；已接受的
Start／Stop／Close 不依赖请求连接继续推进。每个 Scope 最多一个进行中的交换，最多同时协调
32 个 Scope，并按轮转顺序调度；这是传输并发上限，不是接纳配额。失败后从数百毫秒退避到
五秒加 Scope 分散抖动，不耗尽或卸除清理责任。socket I/O 与 bootstrap 投递等待期间不持有宿主日志。

变化的 Run／Scope 观测先持久化再供查询。`last_observed` 是历史证据，与 `coordination`
独立；恢复先标 Pending，guardian 失联标 Unavailable，不擦除旧事实。数字 PID 和历史 Running
不变成启动或发信号权威。尚未尝试 guardian 的 Scope 可在关闭后直接完成而不 exec；
否则必须等待原 guardian 回答，失联不能变成清理成功。

丢弃协调器只停止其传输任务，不终止 guardian；持久意图留在 SQLite，恢复绑定并发现原实例，
不为同一 Scope 重启 guardian。v5 增加空观测表并保留停止／关闭意图；更早格式经过同样的
精确 schema 检查，损坏的查询投影在推进宿主权威前阻断恢复。

`cargo test -p ora-process-runtime --test host_state` 覆盖并发创建、锁竞争、重启去重、锁身份不变、
外部 SIGKILL 后调用者内存丢失、缺失／外来文件、路径长度、权限、链接、版本／schema／身份损坏、
代次耗尽、冲突修复及精确 v1 至 v5 升级（含拒绝时旧令牌和代次不变）。Run 测试覆盖重启发现、Scope／参数不变、提交失败、帧上限及索引负载损坏。强杀 fixture 改变 child 的 HOME 和 cwd，仍使用同一显式状态路径。
测试在测试用户 home 下建立私有临时目录；生产代码不会从该环境变量推导路径。

真实 app 的启动、拒绝、启动方强杀及发现证据见 [guardian 启动](../guardian.zh.md)。
其中已包含持久宿主接管与 guardian 侧 Run 接受；协调测试覆盖派发前取消、恢复后真实副作用去重、
guardian 消失后事实保留，以及失联 Scope 不阻断独立工作。host app 已实现，Git／Node 接入仍待实现；
Controller 授权已推迟，没有 ADR 被标为 implemented。
