# Agent 停止责任

Agent 启动使用 `session-attempts` 下独立并刷盘的尝试锁。插件和每个由宿主启动的子进程继承锁描述符；
刷盘的 PID、启动时间和 boot 线索也防止关闭继承描述符的子进程被误判为停止。关闭时先禁止继续 spawn，
再等待退出。发送信号、超时、单独取得空闲锁都不是完整停止证据。未确认的尝试在重启后仍保留包租约、
工作负载视图和 checkout 责任。终止事件和 Revision 准入等待停止确认及资源释放。

`node.session_stop.confirmation_timeout_ms` 默认为 600000，必须为正数。首次期限持久化，重试不会延长。
跨 boot 证明需要显式配置且与原宿主匹配的 `trusted_host_id`，或由部署提供存储隔离保障的
`single_mount_with_storage_fencing: true`。不会把容器 machine-id 推断为可信宿主身份。schema 14
迁移前盘点旧的运行中 Agent execution，将其保留为 `legacy_unknown`。

root 可向运行中的 Node 请求核对证据：

```sh
ora-node reconcile-session-stop /absolute/node/home EXECUTION ATTEMPT REQUEST_ID maintenance recheck
```

原因可选 `maintenance`、`probe-capacity`、`legacy-upgrade`；模式可选 `recheck`、`after-host-reboot`。
私有 socket 使用内核提供的对端 UID 认证。同一请求 ID 和输入返回已存结果，冲突复用被拒绝。
请求、原始身份、观察和结果均有审计，但不包含命令、Settings 或凭据。确认停止不会直接报告资源已释放。
中断的审计重新获取证据。已确认且资源已释放的明细在七天后、没有 outbox 或未完成审计引用时清理，
保留汇总和审计；未确认明细不按时间自动清理。

Node home 和独立工作负载包视图必须位于支持的本地文件系统及同一设备上。启动时检查硬链接能力，
创建视图时拒绝跨设备复制。精确 MCP 版本租约允许另一已验证的 MCP 版本并存，但禁止修复或删除在用字节。
其他插件类型仍按整个 ID 互斥，改变包类型不能绕过活动或重启后保留的租约。
