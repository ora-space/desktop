# 设置

[English](settings.md) | 中文

设置是保存在 SQLite `user_config` 键值表中的用户偏好。`ora-db` 只读写原始值；类型化的解释、默认值和校验属于应用层（`crates/application/src/user_config.rs`），worktree 根目录则属于 Backend。Backend 的 `Settings` 接口把持久化安排在阻塞执行器上执行，格式错误的存储值会显式失败，而不是被强行转换。前端不保存任何偏好的副本。

| Key | 含义 | 操作 |
|---|---|---|
| `developer_mode` | 显示仅供开发者使用的控件 | `getDeveloperMode`、`setDeveloperMode` |
| `log_level` | 进程级运行时日志级别 | `getRuntimeLogLevel`、`setRuntimeLogLevel` |
| `network_proxy_settings` | 可选的宿主级 HTTP 代理 | `getProxySettings`、`setProxySettings`、`clearProxySettings`、`checkProxySettings` |
| `worktree_root` | 新任务 worktree 的创建根目录 | 原生命令 `get_worktree_root`、`set_worktree_root` |

## 开发者模式与开发者选项

"开发者选项"分类始终可见，因此总能找到开关。初次读取失败时开关保持禁用，仅供开发者使用的控件保持隐藏，页面提供重试；更新失败时保留上一次的 backend 响应。

开发者模式开启后，页面会显示运行时日志级别选择器和"下载日志"按钮，在提供该能力的宿主上通过原生保存对话框导出当天的日志。关闭后两者都会隐藏并卸载，页面不跳转。插件的日志级别和插件日志下载出现在插件管理菜单中，条件相同，见[插件日志](../plugins/plugin-logging.zh.md)。

开发者模式只影响可发现性，不授予任何权限，也不改变传输层授权。

## 日志级别

修改立即作用于当前进程，并持久化到 `user_config.log_level`。选择器显示权威的生效级别，Trace 和 Debug 带有日志量大的警告。启动时恢复持久化的级别，只有未设置时才默认为 `info`；环境变量不起作用。更新顺序和回滚见[日志](../core/logging.zh.md#desktop-配置)。

## 网络代理

代理由主机、端口和可选的 HTTP Basic 凭据组成，存为一个 JSON 值。选择使用它的 marketplace 流量（每个源的 Git 同步和制品下载，见 [Marketplace](../plugins/marketplace.zh.md)）和 Desktop 更新器会使用它。`checkProxySettings` 用请求中给出的设置（与已保存的无关）探测一个绝对的 HTTP(S) URL：任何 HTTP 响应，包括 4xx 和 5xx，都表示代理已经连到了主机；超时、拒绝连接和无效 URL 返回 `unreachable`，而不是让命令失败。

## Cloud 连接

服务器地址（规范化的 origin）存在 `user_config.cloud_server`；登录令牌不在这里，而在操作系统的安全存储里。地址规则、登录和令牌见 [Desktop 宿主](desktop-host.zh.md#cloud-连接)。

## worktree 根目录

worktree 根目录在 设置 → 数据与隐私 中，必须是已存在目录的绝对路径，默认为 `~/.ora/worktrees`。它只是创建目标，见 [Desktop 宿主](desktop-host.zh.md#持久化路径)。

## Marketplace 源

Marketplace 源在 设置 → 插件 → 管理 marketplace 中管理。对话框行为和源的各个字段见 [Marketplace](../plugins/marketplace.zh.md#源)。
