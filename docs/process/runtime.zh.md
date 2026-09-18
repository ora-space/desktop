# 进程运行体系实现状态

[English](runtime.md) | 中文

[已批准的进程 ADR](../../specs/decisions/node/process/README.md) 正在分批实施。
当前已将内存态生命周期内核和[无需 root 的 Linux 尽力清理 adapter](linux/rootless.zh.md)
接通为[可信本机 guardian Run 闭环](guardian.zh.md)和[独立 Node Git 执行](../node/runtime.zh.md)。
现有 Backend 业务启动入口尚未切换。
现有 `ora-process`、`ora-reaper`、Git 和插件入口保持不变。

Linux 另已加入独立 [Helper 部署预检与认证检查服务](linux/helper.zh.md)，不启用工作负载启动，也不构成平台 adapter。
供可信 helper 代码使用的底层执行前启动门禁已加入，但未开放 IPC，仍待真实特权验收。

## 所有权与行为

- `ora-process-protocol` 拥有本地域类型：运行身份、精确启动参数、纳管选择、停止意图、直接退出事实和清理证据，以及 helper 只读检查类型和有界 MessagePack guardian 发现、管理和 Run 消息。
- `ora-process-client` 只依赖协议，不依赖 runtime 或 SQLite；薄 Linux `ora-process-guardian` app 将启动、host 接管和持久 Run 操作交给 runtime。
- `ora-process-runtime::ScopeRuntime<P>` 拥有单个 Scope 的准入、运行记录与停止期限。
  `Platform` 提供已验证能力、创建时纳管、观测和单 Run 信号。Linux 已有无特权 adapter；受控测试也通过这一边界注入平台事实。
- 创建 Scope 时冻结实际保证。必须强但能力不足时拒绝；明确要求尽力时不能静默提升为强模式。
- 同 RunId 同参数重传返回当前事实，变更参数产生冲突。未知启动不会重试；当前对已证明未启动的尝试也仅重放，不续跑。
- 关闭先封闭准入，再安排收尾；停止单个 Run 不关闭 Scope，也不停止相邻 Run。
  等待、通知后等待、立即强制三类请求只能收紧期限或升级动作。
- 直接退出与后代清理独立。收尾策略通知后代，并在显式宽限期后强制结束；等待全部策略持续管理后代，直到退出或收到停止请求。
- 信号发送成功不证明清理完成。观测或信号失败仍保留责任；直接运行／退出证据可确认未知启动，不再次 spawn。
  退出结果可以变得更精确，但不能被较弱证据覆盖；矛盾的启动／退出观测保持阻塞。

## 调用方责任与待实现边界

调用由可变所有权串行化。调用方必须用单调递增的 `Instant` 驱动 `reconcile`；内核没有后台任务、
定时器、重试退避或基于 Drop 的清理。平台方法必须有界，并在启动未知时仍保留稳定尝试身份。
丢弃内核不提供崩溃恢复。

Linux 已支持[有界内存结果捕获](linux/rootless.zh.md#有界结果捕获)，读取器独立推进，限额属于各 Run；
管道 EOF 与进程清理分别表达。
宿主创建意图已提供可选的[持久日志](host/storage.zh.md)，仅使用显式传入的专用目录。
另已支持[独立 guardian 启动与 Ready 发现](guardian.zh.md)，以及可信本机调用方的 guardian 侧 Run
持久接受、查询、输出和强停。授权与租约已推迟；host Run 意图、重启枚举、自动协调、持久查询投影及[宿主 app](host/service.zh.md) 已实现；其余平台 adapter、
完整 I/O、运行恢复、资源交接和生产接入均待实现。
本批不代表阶段 1 完成，也不证明任何 OS 级纳管保证。

## Guardian 启动基础

Workspace 已升级为捆绑 `rusqlite` 0.40.2／SQLite 3.53.2，包含
[WAL-reset 修复](https://sqlite.org/wal.html)。`ora-db` 测试通过真实池连接查询
`sqlite_version()` 和 `sqlite_source_id()`，要求实际链接的主线引擎不低于 3.51.3；
另覆盖多连接间已提交与未提交数据的可见性、回滚、checkpoint 和文件数据库重新打开。
这是依赖前置条件，不是 guardian 崩溃耐久证据；现有业务数据库 schema 和
`synchronous=NORMAL` 政策不变，host 与 guardian journal 已分别启用 `FULL`。

已批准的[无特权 Guardian 启动决策](../../specs/decisions/node/process/recovery/20260917-rootless-guardian-bootstrap-and-reconnect.md)
已实现第一项基础能力：`ora_utils::fs::LinuxFileLock`。它接收已打开的普通文件，以非阻塞方式尝试
独占加锁，竞争失败返回 `WouldBlock`。克隆复制同一个持锁的打开文件描述；`into_file()` 用于
显式交接给子进程，不经过释放再重抢。描述符默认启用 close-on-exec。
并发的其他 fork 在 exec 前仍可能短暂持有副本，因此关闭本地持有者不承诺立即可以重取锁；
调用方必须观测实际加锁结果。

Drop 只关闭描述符，故意不显式解锁，否则可能同时释放子进程共享的锁；依据是 Linux 的
[flock 生命周期语义](https://man7.org/linux/man-pages/man2/flock.2.html)。调用方仍负责保留原 inode、
可信路径解析及实际本地文件系统能力验证。此工具不创建／删除文件，也不证明数据耐久或业务清理完成。
`try_acquire` 可以给未加锁文件加锁；`adopt_inherited` 则要求该打开描述已持有独占 flock，guardian
再独立验证 Scope／路径绑定。
同用户攻击者、网络文件系统及其他平台不在已验证范围。

`cargo test -p ora-utils --test linux_file_lock` 验证锁竞争、复制后的生命周期、文件内容不变、
close-on-exec（含显式 pre-exec 屏障），以及 exec 后持锁者在启动方被外部强杀后仍保有独占资格。测试通过 pidfd 固定并
终止剩余持锁者，然后验证可以重新取得锁。测试子进程不是 guardian 或宿主实现。
状态目录准入与持久创建意图已提供[宿主所有的实现](host/storage.zh.md)，并独立测试日志持有者被外部强杀。
[真实 app 启动测试](guardian.zh.md) 另已验证继承资格、日志先于 Ready 以及启动方死亡后的原实例发现。
持久宿主接管已同时拦截旧会话查询和本地 Run 执行。Controller 授权与 stdin 暂未接入，没有网络 Run 启动端点。

## 验证

运行 `cargo test -p ora-process-runtime` 和
`cargo clippy -p ora-process-protocol -p ora-process-runtime --all-targets -- -D warnings`。
受控集成测试通过公开运行时接口注入平台事实与时间；Linux 无特权测试另通过就绪握手和有界轮询验证
真实子进程及后代，两者均不修改测试运行器的环境变量。
[核心用例索引](../../specs/test-cases/node/process/README.md) 仍将这些证据记为 `Partial`；
崩溃、持久化、完整身份竞争和强纳管平台权限边界仍需直接验证。
