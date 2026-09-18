# 运行时日志

[English](runtime-logging.md) | 中文

Ora Rust 服务通过 `ora-logging` 初始化共享结构化日志。

## 所有权

`ora-logging` 拥有进程 subscriber、JSON 格式、输出选择、轮转、保留清理及固定进程时区。运行时组合根负责显式配置、初始化、保留 `LoggingGuard` 并组合过滤器控制和偏好存储。写入 worker 在 guard 生命周期内存活；提前释放会丢失缓冲输出。关闭时仅有限等待慢输出，stdout 和文件通道满时对调用方施加背压。

请求边界和基础设施发出结构化事件，不配置输出或读取环境变量。应用 handler 和 repository adapter 不重复记录通用完成或仅传播失败的事件。进程初始化必须先于时钟访问；文件准备失败返回类型化错误，不悄悄降级。

插件进程自身的诊断不是运行时日志事件：Host 把它们按插件持久化到独立的 JSONL 文件，只复用本文的事件形状。只有关于插件的 Host 事实（包括插件日志管线自身的故障）才进入运行时日志。见[插件日志](plugin-logging.zh.md)。

## Desktop 配置与决策

存储打开前，临时日志级别明确为 `info`。随后恢复 `user_config.log_level`，仅未设置时采用 `info`。真实读取失败或持久化值损坏中止启动，不伪装成未配置。Desktop 不读取 `ORA_LOG_LEVEL`，包括非法遗留值，也不存在启动覆盖。

2026-09-12 Eric 确认：既然软件内可以动态调整日志级别，就不应再读取环境变量。当前 Desktop 以软件内设置和持久化偏好作为入口。Settings 只拥有偏好持久化；runtime manager 拥有进程过滤器、生效协调和失败回滚。本决策不推导未来 Controller/Node 的完整配置方案，不改变编译期日志上限。

`ora-runtime-settings` 串行执行更新：先重载过滤器，再保存偏好；保存失败回滚过滤器。即使 Tauri 请求取消，已开始的提交或补偿仍完成。日志在 `app_data_dir/logs/ora.log` 每日轮转，保留三个文件；debug 同时输出 stdout，时区来自操作系统。带诊断请求 ID 的错误提示及开发者设置提供下载日志，经原生保存对话框复制当天日志，不向前端暴露私有路径。见 [Desktop 运行时](desktop-runtime.zh.md)。

## JSON 事件

每行一个 JSON 对象，必含 `timestamp`、`level`、`target`、`message`；按需包含 `method`、`span`、`trace_id`、`request_id`。业务元数据归入 `context`，失败详情归入 `error`。`error.`、`context.` 前缀决定字段归属；其他非保留字段默认归入 `context`，空对象省略。

RFC 3339 时间戳采用固定进程时区和偏移；文件名和每日轮转仍按 UTC 边界，本地日期与文件后缀不同时以事件时间戳为准。

## 请求完成与错误

长流延迟完成，直到正常结束、类型化错误、调用方取消或 Desktop channel 丢失。channel 丢失记为 `cancelled`。每个请求使用同一 request ID 关联完成事件和公开错误。业务拒绝与基础设施故障区分；存储或 Git 查询失败不能当作普通任务状态。仅 Git 确认工作树不存在才是 `Conflict`。

`RequestLifecycle` 最后一个句柄释放仍未完成时，在 DEBUG 记录 `abandoned`，避免只有开始事件。`ErrorReport` 在 debug/release 都保留来源链文本；遍历最多 1,024 个节点，循环链在该上限饱和。显式使用的清理和脱敏工具仍保留，但 `from_error` 不调用它们。调用方仍不得记录绝对路径、完整 Git 参数或 remote、SQL 值、提示词、环境值、凭据和无限 stderr。

Git 清理由持久化 `git_cleanup` 后台 worker 执行，其结果不影响主请求错误链；独立日志记录重试、退避和人工处理状态。启动、迁移及状态转换也独立记录。JSONL/Logdy 仍为本地查看工具，OTLP、Loki、Tempo、Grafana 的部署职责不变。

## 发出事件与 Git 桥接

使用 `ora_logging::ora_trace!`、`ora_debug!`、`ora_info!`、`ora_warn!`、`ora_error!`，自动附加当前函数 `method`。关联 span helper 将 span、trace ID 和 request ID 传入嵌套事件，显式事件字段优先。时间使用 `ora_logging::clock::now_local`。

Gitlancer 的框架无关 logger 通过 `OnceLock` 注册一次，未注册时无副作用。执行前记录命令，结束后记录耗时、退出码及成功状态；spawn 失败记录零耗时。Ora 桥接只保留安全的子命令；`git config` 额外记录动作、scope 和安全的 `user.*` key，不记录值、cwd 或其他参数。成功用 info，失败用 error。运行时在日志初始化后注册桥接。

## 测试

`with_trace_logging` 和 `with_recorded_trace_logging` 安装线程作用域 TRACE dispatcher。普通测试和日志断言测试只要触及相同 callsite 都应使用，避免 tracing 兴趣缓存导致并发偶发失败。

Desktop 日志启动测试在隔离子进程中执行，因为 `init_logging` 拥有进程全局时钟及 subscriber。每个子进程执行与 `bootstrap_desktop` 共用的 `initialize_desktop_logging`，在打开存储前检查真实过滤器为 `info`，再通过生产启动路径恢复偏好。每种遗留环境值（包括非法值和空值）均运行两个进程：首次验证未配置默认值并通过 runtime manager 保存 `warn`，第二次从同一数据库验证恢复。只配置子进程环境。子进程专用测试由普通测试运行器忽略，再由父回归测试显式调用。
