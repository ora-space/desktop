# 插件日志

[English](plugin-logging.md) | 中文

每个插件进程自己的诊断由宿主持久化到该插件的 JSONL 日志中，与 Ora 的运行时日志分开。决策是[宿主拥有的插件日志](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/logging/0-host-owned-plugin-log.md) ADR，核心测试用例在 [`test-cases/desktop/plugin/logging/`](https://github.com/ora-space/specs/tree/main/test-cases/desktop/plugin/logging)。

## 职责划分

- **stdout 只承载插件协议**：日志路径上没有任何东西写它。
- **stderr 是所有级别的诊断通道**：SDK 写带版本的信封；第三方库和原生代码可以写任何东西。stderr 不代表 `ERROR`。
- **Ora 运行时日志记录宿主关于插件的事实**：发现、安装、启动、退出、协议和生命周期故障，以及插件日志管线本身的健康状况。插件自己的语句从不进入那里。
- **插件日志记录插件自己的语句**：`ora-plugin-runtime` 拥有管线（`src/plugin_log.rs`）；`ora-plugin-lifecycle` 拥有每个插件的级别（`src/log_levels.rs`）、宿主会话身份和存储边界；`packages/plugin-sdk` 拥有 logger 和 `console.*` 映射（`src/logger.ts`）。通用部分（独占锁、不跟随链接的打开、行尾探测、有界分帧、无损字节渲染）在 `ora-utils` 中。

## 位置与身份

```text
~/.ora/plugins/
  data/<namespace>/<name>/                                 插件数据（ora/storage 的根目录）
  logs/<namespace>/<name>/plugin.log                       当前日志（JSONL）
  logs/<namespace>/<name>/plugin.log.lock                  写入锁（空的附属文件）
  logs/<namespace>/<name>/plugin.recovered-<stamp>[-n].log 保留下来的未结束尾部
  log-levels.json                                          每个插件的级别
```

日志目录按规范插件 id 划分，所以每一代进程、每个已安装的版本都追加到同一个文件；插件发送的任何内容都不能选择路径。`ora/storage/*` 在数据目录树下解析，所以任何存储路径都到不了日志，协议和 SDK 也都没有读日志的方法。sink 要求 `plugins/logs` 是一个普通目录，并逐级创建下面的目录；任何一级已经是文件、符号链接、junction 或 reparse point 都视为冲突，sink 拒绝打开（不替换也不删除任何东西），stderr 仍会被读空。当前文件以不跟随链接的方式打开。

**只有一个写入方**：在接触 `plugin.log` 之前，一代进程以不等待的方式获取 `plugin.log.lock` 上的独占锁，并在写入方的整个生命周期内持有。拿不到锁的一代以 sink 不可用的状态运行：它读空 stderr，把记录计为 `sink_failed`，但从不在另一个写入方旁边写文件。锁放在附属文件上，是因为 Windows 对 `plugin.log` 的范围锁会挡住只读的"下载日志"复制。

**部分写入后的重新打开**：flush 不是 `fsync`，所以崩溃可能让最后一行没有换行符。拿到锁之后，sink 只读最后一个字节：空文件或以换行结尾的文件直接追加；以半行结尾的文件被重命名（从不截断）为 `plugin.recovered-<本地时间戳>.log`，然后创建新文件。重命名失败会让 sink 不可用，文件保持原样。

## 传输格式

SDK 每条记录写一行：前缀 `@ora/plugin-log/v1 ` 加一个紧凑的 JSON 对象，必填 `level`（`TRACE`、`DEBUG`、`INFO`、`WARN`、`ERROR`）和 `message`，可选 `target`、`method`、`context` 和 `error` 对象。`message` 中的换行被转义，写入总是完整完成，所以记录不会交错。

`plugin.logger` 提供 `trace`、`debug`、`info`、`warn`、`error` 和 `child({ target, context })`。`run()` 以默认传输启动时，会在任何协议帧之前接管 `console.debug`（`DEBUG`）、`console.info` 和 `console.log`（`INFO`）、`console.warn`（`WARN`）以及 `console.error`（`ERROR`），并且从不撤销。无法接管 console 时，`run()` 在写任何东西之前抛错。`run()` 之前的输出、更早捕获的方法、其他 worker 或进程以及直接写 stdout 都不在保证范围内。Error、循环引用、`BigInt`、会抛错的 getter 和超大值会降级为有界的描述；记录日志从不向插件代码抛错。

宿主增量地解码 stderr。任何不是合法 v1 信封的内容（纯文本、旧的 `[plugin:<level>]` 前缀、没有前缀的 JSON、未知版本、格式错误的 JSON 或校验失败的载荷）都作为原始的 `INFO` 记录持久化，`target = "plugin.stderr"`；无效的信封还会带上 `context.format_failure`。超过 64 KiB 的记录被切成片段（`context.fragment`），无效的 UTF-8 被可逆地转义并标记 `context.encoding = "escaped-bytes"`。

## 持久化的记录

```json
{
  "timestamp": "2026-09-14T10:00:00+08:00",
  "level": "WARN",
  "target": "db",
  "message": "slow query",
  "method": "query",
  "context": { "ms": 12, "plugin_id": "official/example", "host_session_id": "5f1c6b0e-…", "generation": 3 }
}
```

`timestamp` 是宿主收到记录的本地时间（带偏移），不是插件的事件时间，同一代的记录保持接收顺序。宿主最后写入 `context.plugin_id`、`context.host_session_id`（每次宿主启动生成一次）和 `context.generation`（本次会话中的启动次数），在此之前会移除插件提供的所有保留 key（`plugin_id`、`generation`、`host_session_id`、`request_id`、`trace_id`、`span`、`encoding`、`fragment`、`format_failure`），所以身份、关联和解码标记都无法伪造。其余内容都只是插件自己的说法。没有 `target` 的结构化记录使用 `plugin`。

## 每个插件的级别

每个插件有自己持久化的级别，默认 `INFO`，与运行时日志级别和其他插件都无关。它是解码之后施加的下限；原始文本按 `INFO` 计。`getPluginLogLevel` 和 `setPluginLogLevel` 读取和修改它；应用中的控件在 设置 → 插件 → 管理 → 行菜单 → 日志级别，只在开发者模式开启时显示，旁边是"下载日志"（`download_plugin_log`，通过保存对话框复制当前文件）。修改先持久化，再应用到运行中的那一代，不需要重启；持久化失败则什么都不改。更新会获取插件的操作锁，所以在删除数据的卸载之后到达的更新会被拒绝。级别在升级和保留数据的卸载之后仍然有效。

## 背压与故障

stderr 读取器从不等待磁盘。每条记录被渲染后送入一个以 1024 行和 8 MiB 为上限的队列；队列满时丢弃最新的记录。sink 失败（打开、写入或 flush）会让这一代剩下的记录都计为丢失，stderr 继续被读空，同一代内不重试。每一代统计 `accepted`、`queue_rejected`、`sink_failed`、`indeterminate` 和 `format_failures`（`PluginRuntime::plugin_log_stats`）。运行时日志在队列第一次满时记一条 `WARN`，sink 第一次失败时记一条（带失败类别：`path_conflict`、`writer_busy`、`io`、`write`、`flush`），一代结束时记一条汇总，从不为每条丢失的记录各记一行。日志故障从不改变协议状态，也从不终止插件。

## 生命周期

进程退出不等于日志完成。退出之后，日志收尾在一个五秒的共享期限内进行：stderr 读取器（超过四分之三期限就被截断）、队列排空和 flush。收尾会报告 stderr 是否读到 EOF、写入方是否释放了文件，以及还有多少已接受的记录在排队。只有这之后停止或卸载才返回；停止本身仍然成功。错过期限的写入方会一直持有锁直到真正结束，所以这期间新的一代以 sink 忙的状态运行。宿主退出时不经过生命周期停止插件；残留进程由进程 reaper 结束，留下的未结束尾部会在下次重新打开时被另存。

删除数据的卸载先证明没有写入方持有日志（锁被占用时以 `LogWriterActive` 失败），在同一个卷上的一个事务里暂存包、数据和日志目录，在它们只是被暂存的状态下清除级别设置，然后提交。任何一次移动失败（在 Windows 上打开的句柄会让日志目录无法重命名）或清除级别失败，都会回滚一切，插件保持已安装。保留数据的卸载不碰日志目录和级别。

宿主管理的子进程（`ora/childprocess/*`）的输出交给插件，从不自动持久化；插件通过它的 logger 转发自己想要的内容。

读取级别文件失败或存在不支持的身份/级别时，禁止普通编辑并保留原文件；其余有效条目继续生效。日志级别菜单显示受保护的文件路径并禁用编辑，专门的错误区分只读保护与文件系统写入失败。告警指出不支持的条目，但不记录条目的值。修复文件并重启后可恢复编辑。

用户明确选择删除数据卸载时，可以从可读的 JSON 对象中只移除该插件的 key，其他原始 JSON 值（包括未知值）逐字保留。对于损坏或无法读取的文件，无法安全定向清理；卸载会显示待修复路径并回滚已暂存的删除。
写入或清理前，宿主会核对磁盘字节仍与加载时一致；若用户在外部修复或删除了文件，则阻止旧快照写回并要求重启，保留磁盘上的新内容。
