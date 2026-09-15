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
  （`crates/plugin-lifecycle/src/log_levels.rs`）与 storage 边界；`packages/plugin-sdk` 拥有 logger
  与 `console.*` 映射（`src/logger.ts`）。

## 位置与身份

```
<data-dir>/plugins/
  data/<namespace>/<name>/           插件 data tree（`ora/storage/*` 的根）
  logs/<namespace>/<name>/plugin.log Host 管理的活动日志（JSONL）
  log-levels.json                    Host 独占的按插件级别设置
```

日志目录是安装包、插件数据之外的第三棵持久化树，以 canonical Plugin ID 为键，同一身份的每个进程代
和每个安装版本都追加到同一个文件。Plugin ID 由 Host 从已安装包解析，插件发送的任何内容都不能决定
路径。

`ora/storage/*` 的解析根是 data tree，任何合法 storage path 都到不了 logs tree，因此不需要保留名称；
Plugin Protocol 与 SDK 也都不提供读取日志的能力。sink 在 canonical logs root 下逐级创建目录：某一
级已存在为文件、symlink 或 reparse point 即为冲突，sink 拒绝打开——既不覆盖也不删除——本进程代在
日志不可用的状态下运行，stderr 仍被持续读取。活动文件以不跟随链接的方式打开并通过句柄复核。

## 传输格式

SDK 每条记录写一行：前缀 `@ora/plugin-log/v1 ` 加紧凑 JSON object，必需 `level`（`TRACE`、
`DEBUG`、`INFO`、`WARN`、`ERROR`）与 `message`，可选 `target`、`method`、`context`（object）、
`error`（object）。`message` 内的换行经 JSON 转义，因此一条记录始终是一个 stderr 逻辑行。

`plugin.logger` 提供 `trace`/`debug`/`info`/`warn`/`error` 以及 `child({ target, context })`。以默认
transport 调用 `run()` 后，`console.debug` 映射为 `DEBUG`，`console.info` 与 `console.log` 映射为
`INFO`，`console.warn` 映射为 `WARN`，`console.error` 映射为 `ERROR`，全部经同一 logger 发送。Error
对象、循环引用、`BigInt`、抛错的 getter 和超长值都退化为有界描述；记录日志不会向插件代码抛错，
也不会触碰 stdout。

Host 按字节增量解码 stderr，记录不依赖 pipe 读取边界。所有不是合法 v1 envelope 的内容——纯文本、
旧 `[plugin:<level>]` 前缀、无前缀 JSON、未知版本、格式错误的 JSON、校验失败的 payload——以
`target = "plugin.stderr"` 的 raw `INFO` 记录持久化；无效 envelope 另带 `context.format_failure`。
超过 64 KiB 的记录拆为带 `context.fragment`（`sequence`、`index`、`last`）标识的 raw 片段；无效
UTF-8 以可逆方式转义（`\xNN`，反斜杠加倍）并标记 `context.encoding = "escaped-bytes"`。

## 落盘记录

```json
{
  "timestamp": "2026-09-14T10:00:00+08:00",
  "level": "WARN",
  "target": "db",
  "message": "slow query",
  "method": "query",
  "context": { "ms": 12, "plugin_id": "official/example", "generation": 3 }
}
```

`timestamp`、`level`、`target`、`message` 始终存在；`method`、`context`、`error` 按需出现。Host 在
接收时生成 `timestamp`，并最后写入 `context.plugin_id` 与 `context.generation`，覆盖插件在同名键下
提供的任何值。`request_id`、`trace_id`、`span` 永远不会从插件输入提升为顶层字段。未提供 `target` 的
结构化记录使用 `plugin`。

## 按插件的级别

每个 canonical Plugin ID 拥有独立的持久化级别，默认 `INFO`，与 Ora runtime log level 及其它插件
互不影响。级别是 Host 在解码之后应用的最低落盘级别：达到或高于它的记录保持原级别；raw 文本按
`INFO` 计，因此设为 `WARN` 或 `ERROR` 时它会被策略过滤，不计作丢失。

`getPluginLogLevel` / `setPluginLogLevel` 读取与修改它。桌面端入口在设置 → 插件 → 管理插件 →
行菜单 → 日志级别，仅在开发者模式开启时显示；同一菜单提供「下载日志」，经原生保存对话框复制
该插件的活动 `plugin.log`（`download_plugin_log`），不暴露路径。修改先持久化、再发布给正在运行的进程代，因此对下一条记录立即生效，无需重启；持久化失败时
不改变任何状态并返回错误。该设置在升级与保留数据的卸载后保留；删除数据的卸载会清除它，清除失败
则卸载报告失败，而不是报告清理完成。

## 背压与故障

stderr reader 从不等待磁盘：记录经过有界队列（1024）交给阻塞写线程。队列满时丢弃**最新**记录；
sink 故障（打开或写入）使本进程代其余记录计入丢失，stderr 仍继续读取。每个进程代分别累计
`queue_rejected`、`sink_failed`、`indeterminate`（已交给文件但 I/O 错误后无法判定）与
`format_failures`，可通过 `PluginRuntime::plugin_log_stats` 查询。Ora runtime log 在队列首次满、
sink 首次故障时各记一次 `WARN`，进程代结束时记一条带计数的摘要——从不逐条记录，也从不带插件
原文。日志故障不改变插件的协议状态，也不会终止插件。

## 生命周期

进程退出不等于日志完成。进程退出后，runtime 最多等待五秒让 stderr 读到 EOF，然后关闭队列并等待
写线程 flush 与释放文件。之后 `shutdown_and_wait`（因而 `stop_plugin` 与 `uninstall_plugin`）才
返回，因此新进程代不会与旧代同时写入。若 EOF 永不出现（写端被继承）或 flush 失败，期限到达后
结束该代并把剩余内容报告为未知；停止本身仍然成功。

删除数据的卸载在停止释放日志文件之后，把安装包、data tree 与 logs tree 纳入同一个同卷 staging
事务：任一移动失败——Windows 上 `plugin.log` 被占用会使日志目录无法重命名——则回滚此前的移动，
卸载报告失败，安装包、数据、日志与级别设置全部保持原样。保留数据的卸载不动 logs tree 与级别设置。

Host 管理的子进程输出（`ora/childprocess/*`）交给插件，不会被自动持久化；插件通过自己的 logger
转发需要记录的部分。
