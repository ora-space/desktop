# Agent 停止责任

[English](session-stop.md) | 中文
Agent 启动使用 `session-attempts` 下独立并刷盘的尝试锁。插件和每个由宿主启动的子进程继承锁描述符；
刷盘的 PID、启动时间和 boot 线索也防止关闭继承描述符的子进程被误判为停止。会话停止或 MCP 替换 Agent 进程时，Node 先关闭 spawn 闸门，
立即向持久线索标识的原进程组发送 SIGKILL。插件可能在收到 ora/shutdown 前退出。Node 等待生命周期收尾以取得诊断，
并在释放资源前核验停止证据。spawn 闸门关闭后禁用自动重启。发送信号、超时、单独取得空闲锁都不是完整停止证据。未确认的尝试在重启后仍保留包租约、
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

生命周期会话在 load 失败或断连后保留 provider session ID 和上次 active MCP revision，不通过新建
provider session 或回放 transcript 修复 MCP 失败。下一轮可对每个 generation 重试一次，不做周期性刷新重试。
MCP 包版本变化，或首次 setup 后第九次运行期 load，会仅替换本 execution 的 Agent 进程。Node 先记录停止请求并向原进程组发出终止信号，再等待旧 supervisor 退出。旧 supervisor 完成清理后才安装后继进程。替换前准备最新目标，
确认旧进程停止后才注册新尝试；HOME、checkout、model grant 和 ora-git 上下文继续属于原 execution。
等待清理时的新目标合并到最新代际。

宿主健康独立于 ACP 收敛。解析成功后排队一次自动探测；手动 attempt 按成员合并。Node 在会话间轮转调度，
全局最多 8 个、每会话最多 2 个名额，包含保留和重启恢复的清理责任。取得名额后开始十秒预算：握手最多八秒，
其余时间用于清理。stdio 使用会话的精确包视图、checkout、工作负载身份，仅注入显式设置及 PATH、LANG、LC_ALL、TMPDIR 等无授权的运行必需环境；HTTP 使用 Node 身份，
禁用代理与重定向。握手和清理都成功才报告 Healthy。旧代结果不能更新当前健康；未知清理继续保留名额、租约和
持久停止责任。不通过自动失败重试或 TTL 把健康观察变为准入依据。

探测清理超过十秒时只发布一次 `mcp_probe_timeout`，独立的原停止确认期限仍然有效，到期才标记
`stop_unconfirmed`。随后停止自动等待，保留名额和租约；root 显式对账记录停止证明后唤醒资源 owner。
root 可以向 `session-stop.sock` 发送以换行结尾的 `{"operation":"probe_capacity"}`，查询总名额、
执行中数量、cleanup_pending、排队意图及可用名额。会话查询只暴露匿名容量状态及自己的成员。

升级部署要求：Node home 和包/工作负载视图必须位于支持的本地文件系统。新增停止证据检查会拒绝
NFS、9p、virtiofs 上的 home；升级前先排空会话，再将部署拥有的数据迁移到受支持的本地文件系统。
把未确认责任复制到另一台主机不构成停止证明。

正常结束、自动恢复和 root 对账在持久确认后都会唤醒探测 owner。root 的 probe_capacity 还返回具名残留：execution、attempt、state 和原始 bound_at_ms，可直接用于停止对账；会话查询仍只暴露匿名容量。

探测 owner task panic 会使整个 Node 失败退出，保留槽位、精确版本租约和持久责任，由重启恢复证明清理完成；不支持丢失 owner 后继续服务。stop 请求和 kill_all 的结果仅用于诊断，停止证明来自原锁、进程线索或配置允许的可信重启证据。
