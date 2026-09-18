# 无需 root 的 Linux 进程跟踪

[English](rootless.md) | 中文

`ora_process_runtime::LinuxBestEffort` 是接入 `ScopeRuntime` 的首个真实 Linux
adapter。不需要 root、sudo、特权 helper、cgroup 委派、服务安装或独立业务账号；业务沿用调用方身份。
[特权 helper 路线](helper.zh.md) 保留，但当前不优先推进。

## 准入与所有权

adapter 只报告 `BestEffortOnly`。`RequireStrong` 在启动前拒绝；`PreferStrong` 在接受运行前选择
`BestEffort`。收尾结果只能是 `BestEffortComplete`，不能是 `ConfirmedQuiescence`。

构造时检查 procfs 可读性、pidfd 操作和 `waitid(P_PIDFD)` 支持，并拒绝通过忽略 `SIGCHLD` 或
`SA_NOCLDWAIT` 自动回收子进程的环境。受限内核、procfs 挂载或系统调用策略可能导致构造失败；
无需 root 不代表支持所有容器或 gVisor 配置。procfs 必须对应调用方的 PID namespace。

每个 Run 在 exec 前建立独立 session。环境严格来自 `RunSpec.env`，不会隐式继承宿主环境。
`with_discarded_io()` 明确丢弃三个标准流，并在 exec 前拒绝捕获请求。
`with_bounded_output()` 额外支持 `RunSpec.output = OutputPolicy::Capture { stdout_limit, stderr_limit }`；
stdin 仍关闭。

## 有界结果捕获

限额是调用方显式传入的各流字节数，参与 RunSpec 的精确重放身份比较。零限额只允许空输出；
恰好达到上限不算超限。RunSpec 默认政策仍为 `Discard`。
每条捕获管道有独立读取线程，不依赖输出消费者或 reconcile 才能推进。只保留有界前缀；超限后
继续排空但不增加保留量。超限或读取／读取器建立失败，在下一次 `reconcile` 请求强制清理，
即使后代政策为 `WaitForAll` 也如此。调用方仍必须持续驱动 reconcile，读取线程不独立持有进程控制权。

`ScopeRuntime::read_output(run, stream, offset, max_bytes)` 按请求上限复制字节，不消费已保留数据。
返回保留长度、不可撤销的截断标记和 `Open`／`Eof`／`Failed`。偏移超过保留前缀返回错误，不静默跳过。
必须检查两条流：EOF 加截断仍是不完整，读取失败不等于 EOF。数据只在内存中，不代表持久偏移或
插件会话恢复承诺。未知、未启动、丢弃输出的 Run 没有捕获结果。

直接退出、已跟踪对象清理完成、管道 EOF 相互独立。后代可在直接进程退出后持有管道；逃逸后代
甚至可能在尽力清理完成后仍持有管道。关闭 stdout 不代表进程退出。清理后仍能读取输出，直到
Scope／adapter 被丢弃；Drop 取消读取器，不等待写端退出。本批每个捕获 Run 使用两个读取线程，
仅限制各 Run 的保留量，没有 Scope 总配额或数据退休政策。尚无插件背压、日志轮转、stdin、持久化或 guardian 交接。

调用方必须驱动 `ScopeRuntime::reconcile`，并独占子进程回收权。跟踪期间，其他线程或信号处理器
不得回收这些子进程，也不得启用自动回收。直接子进程直到已跟踪对象完成清理才被 reap，
即使已退出也保留 session ID 的身份锚点。启动后获取 pidfd 失败仍保留所有权并报告启动未知；
后续观测重试获取，不再次启动业务。

## 发现、停止与证据

- 扫描原 session 成员，在获取 pidfd 期间固定 proc 目录；已捕获的 pidfd 在成员后续脱离 session
  或被重新托管后仍保留。不从历史 PPID 猜测归属，不向数字进程组广播信号。
- 通知发送 `SIGTERM`，强制发送 `SIGKILL`。持续保留停止意图，后续发现的成员也收到信号。
  发现失败不阻止向已捕获成员发送信号。
- 通过 pidfd 发送信号。只有独占、尚未 reap 的直接子进程，在获取 pidfd 或发信号失败时，
  才允许以 `Child::kill` 兜底强制停止；原失败仍保持可见。
- 只有直接进程和已捕获成员的退出先于一次成功的新扫描、扫描没有发现新身份、已捕获成员仍全部
  退出时，才报告尽力收尾完成并 reap 直接子进程。扫描／信号错误及存活成员均阻止完成。
- Drop 尝试强制清理并启动直接子进程回收线程。这不是完成证明，也不能应对所有者崩溃或被 `SIGKILL`。

在**被发现之前**新建 session 的后代，以及在原 session 外新生的后代，可能逃过跟踪。
pidfd 避免向复用的 PID 错发信号，但不能让发现完备，也不能阻止逃离。这是尽力清理，
不是隔离同用户业务的安全边界。

## 验证与待完成项

以普通 Linux 用户运行 `cargo test -p ora-process-runtime --test linux_best_effort` 和
`cargo test -p ora-utils --test linux_process`。真实进程测试覆盖准入、重传不重复执行、直接退出后后代
存活、Run 隔离、通知／强制升级、已捕获成员 `setsid`，以及 Drop 对直接子进程的清理。
工具测试覆盖 pidfd 退出与回收、过期 proc 观测和非 UTF-8 进程名；没有通过强制数字 PID 复用或
耗尽描述符验证身份复用及启动后获取失败恢复的完整路径。

`cargo test -p ora-process-runtime --test linux_output` 覆盖独立二进制流、精确限额、重放冲突、偏移读取、
无消费者时双流超过管道容量、超限终止与隔离、后代持管道，以及 EOF 先于退出。
共享读取器测试为 `cargo test -p ora-utils --test pipe_capture`，覆盖零／恰好／超限容量、偏移和写端
仍存活时的取消。线程／描述符耗尽及读取错误恢复仍有验收缺口。

持久宿主／guardian 所有权、崩溃恢复、I/O 交接和生产入口仍未实现，详见[运行体系状态](../runtime.zh.md)。
这些测试不完成强纳管 ADR，也不能从 Linux 结果推导 Windows／macOS 支持。
