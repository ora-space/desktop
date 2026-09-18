# 插件日志

[English](plugin-logging.md) | 中文

每个插件进程的诊断信息由 Host 持久化到该插件自己的 JSONL 文件，与 Ora runtime log 分离。设计
依据见 `specs/decisions/desktop/plugin/logging/0-host-owned-plugin-log.md`，核心测试用例位于
`specs/test-cases/desktop/plugin/logging/`。

## 归属边界

- **stdout 只承载 Plugin Protocol。** 日志路径上没有任何东西写 stdout。
- **stderr 是所有级别的诊断 transport。** SDK 写版本化 envelope；第三方库和原生代码可能写任意内容。
  stderr 不代表 `ERROR`。
- **Ora runtime log 记录 Host 事实**：发现、安装、启动、退出、协议与生命周期故障，以及插件日志管线
  自身的健康状况。插件自己的陈述不会进入。
- **Plugin log 记录插件自己的陈述。** `ora-plugin-runtime` 拥有管线
  （`crates/plugin-runtime/src/plugin_log.rs`）；`ora-plugin-lifecycle` 拥有按插件的级别设置
  （`crates/plugin-lifecycle/src/log_levels.rs`）、Host session 身份与 storage 边界；
  `packages/plugin-sdk` 拥有 logger 与 `console.*` 映射（`src/logger.ts`）。通用部件——排他锁、
  不跟随链接的打开、行尾探测、有界行分帧、无损字节渲染——位于 `ora-utils`
  （`crates/utils/src/fs`、`crates/utils/src/text`）。

## 位置与身份

```
<data-dir>/plugins/
  data/<namespace>/<name>/                    插件 data tree（`ora/storage/*` 的根）
  logs/<namespace>/<name>/plugin.log          Host 管理的活动日志（JSONL）
  logs/<namespace>/<name>/plugin.log.lock     writer 锁（sidecar，空文件）
  logs/<namespace>/<name>/plugin.recovered-<stamp>[-n].log   被保留的残缺尾部
  log-levels.json                             Host 独占的按插件级别设置
```

日志目录是安装包、插件数据之外的第三棵持久化树，以 canonical Plugin ID 为键，同一身份的每个进程代
和每个安装版本都追加到同一个文件。Plugin ID 由 Host 从已安装包解析，插件发送的任何内容都不能决定
路径。

`ora/storage/*` 的解析根是 data tree，任何合法 storage path 都到不了 logs tree，因此不需要保留名称；
Plugin Protocol 与 SDK 也都不提供读取日志的能力。sink 要求 `plugins/logs` 本身是普通目录，并在其下
逐级创建：某一级已存在为文件、symlink、junction 或其它 reparse point 即为冲突，sink 拒绝打开——既
不覆盖也不删除——本进程代在日志不可用的状态下运行，stderr 仍被持续读取。活动文件以不跟随链接的方
式打开并通过句柄复核。

### 同一活动文件只有一个 writer

在触碰 `plugin.log` 之前，进程代先对 sidecar `plugin.log.lock` 取得排他 advisory 锁
（`ora_utils::fs::ExclusiveFileLock`，基于 `File::try_lock`，从不等待）。锁在 writer 存活期间一直
持有，writer 线程结束或其进程退出时释放，因此同时排除了尚未释放文件的旧进程代和共用同一 home 的
另一个 Ora Host。拿不到锁的进程代在 sink 不可用的状态下运行：继续 drain stderr，把每条通过过滤的
记录计入 `sink_failed`，但绝不在另一个 writer 旁边打开文件。锁放在 sidecar 而不是 `plugin.log` 本身，
是因为 Windows 的区间锁会同时阻塞插件运行期间「下载日志」的只读复制。

### 部分写入后重开

正常 flush 只是把用户态缓冲交给操作系统，不是 `fsync`；崩溃或被 kill 都可能让最后一行没有换行。
取得 writer 锁后，sink 只读取文件的最后一个字节（`ora_utils::fs::classify_line_tail`）。空文件或
以换行结束的文件正常追加。以半行结束的文件被重命名——不截断、不覆盖——为同目录下的
`plugin.recovered-<本地时间戳>.log`（名称被占用时追加 `-1`、`-2`、…），再新建 `plugin.log`。Host 把
恢复作为事实记录一条日志但不复制任何内容；恢复失败（重命名被拒绝）则 sink 不可用，残缺文件原样
保留。恢复文件属于插件的日志目录，随卸载的 retain/delete 处置。该机制只隔离尾部，不是 rotation。

## 传输格式

SDK 每条记录写一行：前缀 `@ora/plugin-log/v1 ` 加紧凑 JSON object，必需 `level`（`TRACE`、
`DEBUG`、`INFO`、`WARN`、`ERROR`）与 `message`，可选 `target`、`method`、`context`（object）、
`error`（object）。`message` 内的换行经 JSON 转义，因此一条记录始终是一个 stderr 逻辑行。stderr sink
同步写入，短写会继续直到整条 envelope 写完，因此 SDK 自己的两条记录不会交错。

`plugin.logger` 提供 `trace`/`debug`/`info`/`warn`/`error` 以及 `child({ target, context })`。以默认
transport 调用 `run()` 时，SDK 在改变任何状态、写出第一个协议帧之前接管且只接管五个 console 方法：
`console.debug` 映射为 `DEBUG`，`console.info` 与 `console.log` 映射为 `INFO`，`console.warn` 映射为
`WARN`，`console.error` 映射为 `ERROR`，全部经同一 logger 发送。接管不会被撤销：它跨越初始化失败、
`ora/shutdown` 和每个回调的结束，之后调用全局方法的第三方依赖同样进入 logger。若 console 无法接管
（方法被冻结），`run()` 在写出任何内容前抛错，插件不进入协议运行。`run()` 之前的输出——模块求值、
`createPlugin`、`registerMethod` 的函数体——保持 runtime 原行为；接管前缓存的方法、其它 worker 或
进程、其它 console 方法以及直接写 stdout 都不在保证内。Error 对象、循环引用、`BigInt`、抛错的 getter
和超长值都退化为有界描述；记录日志不会向插件代码抛错，也不会触碰 stdout。

Host 按字节增量解码 stderr，记录不依赖 pipe 读取边界。所有不是合法 v1 envelope 的内容——纯文本、
旧 `[plugin:<level>]` 前缀、无前缀 JSON、未知版本、格式错误的 JSON、校验失败的 payload（类型错误、
标识符超过 256 字节、`context` 或 `error` 嵌套超过 16 层）——以 `target = "plugin.stderr"` 的 raw
`INFO` 记录持久化；无效 envelope 另带 `context.format_failure`。超过 64 KiB 的记录拆为带
`context.fragment`（`sequence`、`index`、`last`）标识的 raw 片段；无效 UTF-8 以可逆方式转义
（`\xNN`，反斜杠加倍）并标记 `context.encoding = "escaped-bytes"`。

## 落盘记录

```json
{
  "timestamp": "2026-09-14T10:00:00+08:00",
  "level": "WARN",
  "target": "db",
  "message": "slow query",
  "method": "query",
  "context": {
    "ms": 12,
    "plugin_id": "official/example",
    "host_session_id": "5f1c6b0e-…",
    "generation": 3
  }
}
```

`timestamp`、`level`、`target`、`message` 始终存在；`method`、`context`、`error` 按需出现。
`timestamp` 是 Host 用 `ora_logging::clock::now_local` 取得的接收时间，RFC 3339 且带本地时区偏移——
不是插件事件时间，也不是磁盘写入时间；同一进程代的记录按接收顺序落盘，即使墙上时钟回拨。Host 最后
写入 `context.plugin_id`、`context.host_session_id` 与 `context.generation`：session id 在每次 Host
启动时生成且不复用，generation 是该 session 内该插件的启动计数，因此两次 Host 运行都把插件启动为
第 1 代时仍可区分。写入身份之前，Host 会清除插件在 `context` 下提供的全部保留键——`plugin_id`、
`generation`、`host_session_id`、`request_id`、`trace_id`、`span`、`encoding`、`fragment`、
`format_failure`——身份、关联与 Host 自己的解码标记都无法伪造。记录中的其它内容，包括 `level`、
`target`、`method`、`context` 与 `error`，仍是插件自己的陈述。未提供 `target` 的结构化记录使用
`plugin`。

## 按插件的级别

每个 canonical Plugin ID 拥有独立的持久化级别，默认 `INFO`，与 Ora runtime log level 及其它插件
互不影响。级别是 Host 在解码之后应用的最低落盘级别：达到或高于它的记录保持原级别；raw 文本按
`INFO` 计，因此设为 `WARN` 或 `ERROR` 时它会被策略过滤，不计作丢失。

`getPluginLogLevel` / `setPluginLogLevel` 读取与修改它。桌面端入口在设置 → 插件 → 管理插件 →
行菜单 → 日志级别，仅在开发者模式开启时显示；同一菜单提供「下载日志」，经原生保存对话框复制
该插件的活动 `plugin.log`（`download_plugin_log`），不暴露路径。修改先持久化、再发布给正在运行的
进程代，因此对下一条记录立即生效，无需重启；持久化失败时不改变任何状态并返回错误。更新在该插件的
operation lock 下执行——与卸载持有的是同一把锁——因此与卸载有确定顺序：在删除数据的卸载提交之后
到达的更新会发现该身份已不再安装而被拒绝，不会重建刚被清除的设置。该设置在升级与保留数据的卸载后
保留；删除数据的卸载会清除它（见「生命周期」）。

## 背压与故障

stderr reader 从不等待磁盘。它把每条通过过滤的记录渲染为 JSON 行，交给一个同时按条数（1024 行）
和字节数（8 MiB 已渲染行）限界的队列；64 KiB 全为无效字节的极端 raw 记录渲染后最多膨胀到八倍，
真正保证内存有界的是字节上限。队列满——两种度量任一——时丢弃**最新**记录。sink 故障（打开、写入或
flush）使本进程代其余记录计入丢失，stderr 仍继续读取；同一进程代内不重试 sink。每个进程代分别累计
`accepted`、`queue_rejected`、`sink_failed`、`indeterminate`（已交给文件但 I/O 错误后无法判定——
失败的那次写入以及上次成功 flush 之后缓冲的所有行）与 `format_failures`，可通过
`PluginRuntime::plugin_log_stats` 查询。Ora runtime log 在队列首次满、sink 首次故障时各记一次
`WARN`（附故障类别：`path_conflict`、`writer_busy`、`io`、`write`、`flush`），进程代结束时记一条带
计数的摘要——从不逐条记录，也从不带插件原文。日志故障不改变插件的协议状态，也不会终止插件。

## 生命周期

进程退出不等于日志完成。进程退出后，runtime 在 stderr reader、队列处理和 flush **共用的一个**五秒
截止时间内收尾：reader 得到前四分之三去读到 EOF，到期即被切断并关闭队列；writer 在同一截止时间的
剩余部分内 drain、flush 并释放文件。任何阶段都不会重新获得完整超时。收尾分别报告三件事：stderr
是否读到 EOF（否则有数量未知的未读输出被放弃——写端被继承是常见原因）、writer 是否释放了文件，以及
`queued_at_deadline`——到期时已入队但 writer 尚未取走的记录数。之后 `shutdown_and_wait`（因而
`stop_plugin` 与 `uninstall_plugin`）才返回；停止本身仍然成功。

错过截止时间的 writer 是阻塞线程，无法中止，但也不会被遗忘：它一直持有 writer 锁直到真正结束。
期间新进程代可以启动，但会发现 sink 被占用，只 drain 并计数，不在旧 writer 旁边写入；在释放之后
启动的进程代正常持久化。异常退出与未能就绪的启动走同一收尾。目前 Host 退出不会经生命周期停止
插件——进程 reaper 直接终止残留进程——因此被这样结束的插件可能留下残缺尾部，由下一代重开时按上文
规则隔离。

删除数据的卸载先证明没有 writer 持有该插件的日志（取得并释放 writer 锁；锁被占用则在移动任何东西
之前以 `LogWriterActive` 失败），然后把安装包、data tree 与 logs tree 纳入同一个同卷 staging 事务，
再在三棵树仍只是 staged 的状态下清除级别设置，最后才提交。任一移动失败——Windows 上 `plugin.log`
被占用会使日志目录无法重命名——或级别清除无法持久化，都会回滚此前的移动并报告卸载失败，安装包、
数据、日志与级别设置全部保持原样，插件仍处于已安装状态。保留数据的卸载不动 logs tree 与级别设置，
也无需 writer 证明。

Host 管理的子进程输出（`ora/childprocess/*`）交给插件，不会被自动持久化；插件通过自己的 logger
转发需要记录的部分。
