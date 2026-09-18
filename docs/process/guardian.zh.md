# 无特权 Guardian 与可信本机管理

[English](guardian.md) | 中文

按 Eric 的明确要求，当前信任本机 Node 和管理程序，不使用秘密令牌、签名密钥或 Controller 授权。
私有路径与内核 peer UID 检查防止意外跨用户访问，不隔离恶意同 UID 程序。
宿主实例身份和持久代次仍保留，用于拒绝接管后迟到的旧实例请求。

## 独立所有权

调用方向 [HostState](host/storage.zh.md) 显式提供专用状态目录和可信可执行文件。
不读取 HOME，不要求 root helper 或服务安装。宿主先提交创建意图和已消耗的启动记录，再 exec。
专用 socketpair 传递 bootstrap 身份；原 Scope 独占 flock 直接继承，不先解锁再重抢。
子进程进入独立 session，清空环境，exec 时关闭非显式描述符；命令行只有 --bootstrap。
不安装父进程死亡或 Drop-kill 策略。

Guardian 验证继承的打开描述、Scope 锁 inode、私有路径及本地文件系统。
只有它初始化 WAL/FULL 的 guardian.sqlite，再发布 control.sock、events.sock、io.sock。
Ready 晚于初始化，不证明 Run 已启动或清理完成。启动错误、回复丢失、guardian 死亡都不授权
为同一 Scope 再启动一个 guardian。旧日志、锁和端点保留，不自动修复部分初始化。

## 基于身份的接管

GuardianManagement::bind 接收通过 HostState 独占资格提交的绑定。可信调用方必须遵守该所有者，
不自行编造更高代次。更高绑定先提交再确认；相同绑定幂等，低代次或同代次不同实例拒绝。
宿主会话只包含公开的宿主绑定，不包含秘密凭据。

请求在执行锁内按日志当前绑定检查，接管前排队的请求也不能沿用旧资格。
Scope 锁保持到剩余 worker 关闭 SQLite；未结束事务或存储失败不能产生成功接管确认。
Ready 发现独立于当前宿主会话查询。

消息使用有界、长度前缀 MessagePack（16 KiB、深度 16），协议版本为 3；每次交互有 5 秒 I/O
期限，各通道 worker 独立限额。Guardian 另以 50 ms 间隔推进生命周期，不依赖 host 保持连接。

## 可信本机 Run 闭环

Ready、bind 后，通过 GuardianRuns::execute 使用协议所有的 GuardianRunOperation：

- Start 必须显式传入不复用的 RunId、精确 RunSpec 和 host_disconnect: KeepRunning。
  Guardian 先提交参数和初始不确定快照，再 exec。同 ID 同参数只返回原尝试；改参数冲突。
  即使证明 exec 失败，也不能用该 ID 重试。环境值原样存于私有日志，不写日志输出；
  不要把数据库当作已经脱敏的产物。
- Query 返回日志快照，分别表达直接退出与尽力清理。回复丢失不代表未接受；
  exec 后存储失败仍保留原尝试，不再次执行。
- Stop 对单个 Run 请求强制清理；Close 先持久封闭准入，再强制清理。
  两者可以重复调用；关闭后仍可查询或重放已有尝试。信号投递不是终态，
  应通过 Query／Scope 查询清理证据。
- Output 通过 io.sock 分别读取 stdout／stderr 前缀。RunSpec 必须显式开启捕获：
  每条流最多 1 MiB、单次读取最多 4096 字节、每个 Scope 最多 64 个已接受尝试（含已终结尝试）。
  超额拒绝，不自动删记录或淘汰输出。截断和 EOF 显式返回；字节只在内存，随 guardian 消失。
- Control 承载 Start／Query／Stop／Close／Scope；events.sock 暂无订阅。
  所有 Run 请求在接管执行锁内检查当前公开宿主绑定。stdin 关闭；
  host 断连执行 KeepRunning，不推导租约到期。

工作负载复用 LinuxBestEffort adapter 与 pidfd 观测，处于独立 session，exec 时关闭非显式描述符。
发现前就脱离的后代可能逃逸。Guardian 被 SIGKILL 后，工作负载可能继续存活：
旧数字 PID 不重新获得发信号资格，日志中 Running 只是历史事实，不是实时证据，
也不会重启 guardian 或 Run。host 和[独立 Node](../node/runtime.zh.md) 已将此闭环用于受管 Git；
Controller IPC 和现有 Backend 业务启动入口仍不在本次集成范围内。

## 已有文件与版本

RunSpec 新增显式 `TerminateOnOwnerExit` 存活策略。Linux adapter 在 exec 前固定观测到的所属进程，
对应 pidfd 退出后强制收尾该 Run，不依赖 host 连接。数字身份不用于发送信号，只是清理触发条件，
不是资源恢复的证明；调用方仍须查询原 Run 的清理结果。默认仍为 `Independent`，缺少字段的旧持久
参数沿用该策略。Wire v3 拒绝旧存活对端；应保留其兼容管理程序与日志，而非替换原 guardian。

新 host 日志为版本 6，guardian 日志为版本 4，各自保存所属 Run 日志，均不再有凭据列。Host 恢复事务化迁移精确 v1/v2/v3/v4/v5 布局，
保留原 Scope、代次、锁 inode 与已消耗启动尝试。v2 host 若已有 Scope 目录，迁移前即拒绝，
保留旧版本管理所需的原令牌和代次。此时应使用兼容旧 host，或为新任务选择另一个专用状态目录，
不能通过删除旧 Scope 强行升级。删除旧令牌列不承诺安全擦除 SQLite 空闲页
或备份中的历史值。未知布局拒绝，不自动修复。

存活旧 guardian 使用旧令牌协议，与新客户端不兼容；必要时使用其兼容管理版本，
不改写它的日志、不覆盖仍有责任的二进制，也不重启旧 Scope。本批不收养旧活进程。

## 验证与边界

真实 app 测试覆盖独立 session、启动方 SIGKILL 后存续与接管、原实例发现、失败 exec 去重、
错误 Scope／UID、继承锁资格、旧日志拒绝、三个旧通道失效、迟到字节及接管回复丢失。
Runtime 测试覆盖执行队列检查、持久化失败与锁生命周期；Host 测试覆盖精确 v1/v2 迁移。
Run 真实 app 测试覆盖精确重放、退出／输出、Start 回复丢失、exec 前后存储失败、host SIGKILL
后从 host 日志找回 Run 并接管、强停／关闭，以及 guardian SIGKILL 后工作负载仍活着但不持有 Scope 锁。

Controller 授权和租约已推迟。host 自动协调与持久查询投影已通过[宿主 app](host/service.zh.md) 提供本机 IPC；stdin、持久输出、
插件接入和 guardian 死亡恢复仍未完成。Node／Git 已接通。强纳管、服务管理器下存续、物理断电与其他平台支持尚未证明。
