# Node 插件安装

[English](plugin-installation.md) | 中文

Workspace 的插件安装由 Cloud 冻结计划、Controller 下发、Node 执行。Controller 从 operation 快照读取
`plugin_input`，原样登记 execution；它不查询市场、不解析版本，也不再创建插件 Substrate effect。
Node 握手必须包含 `plugin_install`。完成的逐项结果交给 Cloud 后，Controller 才推进 `plugin` 步骤；
单项失败也是完成结果，整体失败进入 `PLUGIN_EXECUTION_FAILED` 重试等待。

## 执行和目录

云端使用带运行许可的 `ControlledPlugins`，内部命令为 `InstallPlugins` 或 `RemovePlugins`。
Node 在受理和实际开始执行时检查许可，先持久化原始输入，再由独立线程下载和解包，控制会话仍可心跳和查询。
本地未启用运行控制的私有 IPC 也支持原始安装／移除命令；已经启用运行控制的数据目录不能退回该入口。

插件落在显式 `node.home_directory` 下的 `plugins/installed/<namespace>/<name>/<version>/`。
临时下载和解包目录为 `plugins/.node-installs/install-*`，不参与发现。安装复用 `ora-plugin-manager` 的包校验，
要求计划中的 SHA-256、插件身份、版本和 target 匹配；只在校验通过后发布目录。安装不会启动插件。
网络请求有时间和大小上限；临时网络故障至多尝试三次。同版本有效包直接成功，不重复下载。
升级成功后清理旧版本目录。移除不存在的插件成功，移除操作保留插件 data/config。
既有目录含符号链接时拒绝跟随。

部署者可以在 Node 服务 JSON 顶层加入以下配置，让较大的官方包在慢速连接上有足够时间完成下载：

```json
"plugins": { "download_timeout_seconds": 600 }
```

省略 `plugins` 或其中的时限时，单次请求仍限制为原来的 60 秒。配置必须为 10 至 1200 秒的整数；
非法值在启动读取配置时拒绝。包含重试开销的总预算为单次时限的三倍加 70 秒：默认仍是 250 秒，
配置 600 则为 1870 秒。建立连接仍限制为 10 秒，临时失败仍至多重试两次，包大小仍限制为 512 MiB；
失败后从头重传。该部署策略保持 TLS 验证、冻结的下载地址、SHA-256 校验和测试等待时限，
远程插件命令不能指定它。

`PluginInstaller::catalog()` 返回 `DirectoryPluginCatalog`。会话宿主需复用这个实例，先获得 use lease，
再查找精确版本。租约存在时，替换或移除该插件返回 `plugin_in_use`。本变更提供目录与租约实现；
生产控制通道的 Agent session 执行接线属于任务 C 的后续部分。

## 持久化与恢复

SQLite schema v7 增加插件执行与 outbox，并保留旧 worktree/clone 的身份、结果和事件。
输入 identity 相同但内容不同会被拒绝。终态与唯一的 sequence 1 事件在同一事务中提交。
状态查询和重复命令不删除事件；只有准确的 ACK 删除 outbox，终态仍可查询。Controller 先提交 Cloud 的
事件接管再 ACK；通过查询得到结果不构造 ACK。

启动清理本执行器的临时目录。本地未围栏的未完成输入可重跑；云端 Node 重启后旧 incarnation 的许可失效，
保留的未完成执行以 `interrupted` 结算，由 Cloud/Controller 登记新执行并获取新许可。已完成但未 ACK 的事件
按原身份重放，不改写为当前 incarnation。该行为与当前运行控制约定一致，收窄了早期 ADR 的无条件重跑描述。

## 验证

相关入口：`apps/ora-node/tests/plugins.rs`（部署默认值、非法时限拒绝、真实包安装收到配置的下载策略、本地 HTTP）、
`apps/ora-node/tests/standalone/repository_plugins.rs`（实际 Node 进程和 IPC 重启重放）、
`crates/node-db/src/tests/plugin.rs`（事务、重启、运行许可）、
`apps/ora-controller/tests/workspaces/plugins.rs`（假 Cloud gRPC 与假 Node WebSocket 的步骤推进）。

本范围验证 desktop 侧插件执行链路。Cloud 的真实数据库投影和容器中的工作区卷挂载仍需跨仓库 M1 联调。
