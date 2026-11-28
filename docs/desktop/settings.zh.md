# 设置

[English](settings.md) | 中文

设置是保存在 SQLite `user_config` 键值表中的用户偏好。`ora-db` 只读写原始值；类型化的解释、默认值和校验属于应用层（`crates/application/src/user_config.rs`），worktree 根目录则属于 Backend。Backend 的 `Settings` 接口把持久化安排在阻塞执行器上执行，格式错误的存储值会显式失败，而不是被强行转换。前端不保存任何偏好的副本。

| Key | 含义 | 操作 |
|---|---|---|
| `developer_mode` | 显示仅供开发者使用的控件 | `getDeveloperMode`、`setDeveloperMode` |
| `log_level` | 进程级运行时日志级别 | `getRuntimeLogLevel`、`setRuntimeLogLevel` |
| `network_proxy_mode` | 跟随系统、手动或直连 | `getProxySettings`、`setProxySettings`、`clearProxySettings` |
| `network_proxy_settings` | 可选的宿主级 HTTP 代理 | `getProxySettings`、`setProxySettings`、`clearProxySettings`、`checkProxySettings` |
| `network_proxy_check_url` | 用户明确确认的 HTTP(S) 探测网址 | `getProxyCheckUrl`、`setProxyCheckUrl` |
| `worktree_root` | 新任务 worktree 的创建根目录 | 原生命令 `get_worktree_root`、`set_worktree_root` |

## 开发者模式与开发者选项

"开发者选项"分类始终可见，因此总能找到开关。初次读取失败时开关保持禁用，仅供开发者使用的控件保持隐藏，页面提供重试；更新失败时保留上一次的 backend 响应。

开发者模式开启后，页面会显示运行时日志级别选择器和"下载日志"按钮，在提供该能力的宿主上通过原生保存对话框导出当天的日志。关闭后两者都会隐藏并卸载，页面不跳转。插件的日志级别和插件日志下载出现在插件管理菜单中，条件相同，见[插件日志](../plugins/plugin-logging.zh.md)。

开发者模式只影响可发现性，不授予任何权限，也不改变传输层授权。

## 日志级别

修改立即作用于当前进程，并持久化到 `user_config.log_level`。选择器显示权威的生效级别，Trace 和 Debug 带有日志量大的警告。启动时恢复持久化的级别，只有未设置时才默认为 `info`；环境变量不起作用。更新顺序和回滚见[日志](../core/logging.zh.md#desktop-配置)。

## 网络代理

Desktop 默认**跟随系统代理**。也可以选择**手动代理**（填写主机、端口及可选的 HTTP Basic
凭据）或**直连**（忽略系统和环境变量代理）。已有手动配置升级后保持生效。切换模式保留已保存
的手动地址，降级后旧版本仍使用它。主动清空会删除保存的手动配置，并恢复默认模式。

Windows 读取当前用户的 WinHTTP 静态设置。仅开启自动检测时，Ora 直接连接并跳过 WPAD。
仅配置 PAC 地址时，Ora 会提示不支持。Linux 在持久化 GIO 后端可用时读取 GNOME 静态设置，
否则使用 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 和 `NO_PROXY`。首版不支持 Windows
集成代理认证。本机回环和命中系统绕过规则的目标直连。代理解析或连接失败会显示原因，且不会重试直连。

这些规则用于 Desktop 更新及开启「使用代理」的市场源（见 [Marketplace](../plugins/marketplace.zh.md)）。
连接测试使用当前表单模式跟随跳转，每一跳按实际目标选路。最终返回 HTTP 2xx 时显示成功和最终路线，
其他 HTTP 响应显示该状态码。HTTP 407 会提示代理认证失败。
手动代理可输入主机名，也可在主机字段粘贴完整地址。例如，`http://127.0.0.1:7890/` 会回填
主机，端口字段为空时也会回填端口。地址中的凭据会移到用户名和密码字段。默认使用 HTTP，
保留 `https://`、`socks5://` 和 `socks5h://` 前缀。`https://` 表示 Ora 与代理服务器之间使用
TLS。保存时会校验地址并写入规范化后的字段。
验证使用表单当前值，检查更新使用已保存的代理。手动模式下最终直连的请求会提示无法确认手动代理。

## Cloud 连接

服务器地址（规范化的 origin）存在 `user_config.cloud_server`；登录令牌不在这里，而在操作系统的安全存储里。地址规则、登录和令牌见 [Desktop 宿主](desktop-host.zh.md#cloud-连接)。

## worktree 根目录

worktree 根目录在 设置 → 数据与隐私 中，必须是已存在目录的绝对路径，默认为 `~/.ora/worktrees`。它只是创建目标，见 [Desktop 宿主](desktop-host.zh.md#持久化路径)。

## Marketplace 源

Marketplace 源在 设置 → 插件 → 管理 marketplace 中管理。对话框行为和源的各个字段见 [Marketplace](../plugins/marketplace.zh.md#源)。

## 升级与读取失败

代理初次读取失败时设置页显示错误和重试，不提供空白编辑器，避免覆盖原值。后台读取失败时保留当前草稿，保存、清空和连接探测保持禁用，直到重试成功。

旧默认 Agent 选择在偏好迁移时保留。旧版包名只有在已安装 Agent 中存在唯一匹配时才补齐命名空间；歧义或未安装的选择保持原样。

只有新建数据库会请求初始化默认 Marketplace 源。已有空列表（包括旧版本未记录删除标记的空列表）在升级和重启后保持为空。首次初始化也尊重发行版预置的源。

首装来源在执行 schema 步骤前与迁移历史一同提交。迁移或发行版初始化中断后，重启会继续待完成任务；默认源初始化不依赖发行版可选的首次安装 SQL。

连接检查的默认网址为 `https://google.com`，可以自行编辑。失败表示该网址的连通性结果，可能受目标网站或网络限制影响，不判定或改写已保存的代理配置。后台读取失败时，已打开的探测弹窗和网址草稿保持不变，恢复成功前禁止探测。

点击验证会先将 HTTP(S) 网址单独保存到 `user_config`，目标不可达也保留。取消放弃本次草稿，重新打开恢复已保存网址，重启和升级继续保留。只有缺失值使用 Google 默认值；读取失败显示错误，保存失败保留草稿且不执行探测。确认网址不会保存或清空代理凭据。
