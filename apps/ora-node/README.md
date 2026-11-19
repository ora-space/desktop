# ora-node

[English](README.en.md)

Rust Node 是执行程序。Cloud 网络入口要求双向 TLS、固定目标与 root 管理/UID 1000 工作负载分离；管理凭据和本地日志位于受保护路径。持久接受后、首次目录/Git 变更前分别检查资格，恢复不重新开放旧 incarnation（进程启动代次）。已有工作按原范围收尾。明文网络和全局个人 Git 凭据明确拒绝。examples/loopback-transport-fixture 仅用于真实 TLS、Git 和崩溃测试，使用注入的测试目标与绑定，不进入镜像。操作系统身份隔离由 cluster 真实沙盒验收。

启用 Agent 时，同一 checkout 最多同时容纳 8 个未结束会话；恢复前序 Revision 或停止证据未知时仍独占 checkout。`checkout_busy` 和 `checkout_session_limit` 只终结新会话，不关闭控制连接。握手声明 `shared_checkout_sessions`。

设置了 `process.workload_uid` 时，宿主必须设置 sysctl `fs.protected_hardlinks = 1`（否则 Node 拒绝启动，并在每次交接 checkout 之前重新读取；该 sysctl 没有命名空间，容器继承宿主的值），并给 Node 至少 38,000 的软 `RLIMIT_NOFILE`（推荐 65,536）：每次交接遍历至多持有 4,099 个描述符，无论多少 checkout 很深，Node 同时至多运行 9 次遍历，目录嵌套超过 4096 层的 checkout 以 `checkout_too_deep` 拒绝。见[把共享 checkout 交给工作负载用户](../../docs/node/agent-sessions.zh.md#把共享-checkout-交给工作负载用户)。

## 验证与边界

模块测试覆盖真实协议、恢复日志、Git 或 TLS 的所属边界；cloud fixture 及回环程序不证明生产多人授权。真实部署用 cluster Compose，契约源为 third_party/cloud 固定提交 f0b5d9c，生成文件不手改。完整验收与缺口见根 [运行时控制说明](../../docs/controller/runtime-control.zh.md)。

## Session MCP

`session_mcp` 声明支持 `AgentSessionSpec.mcp` 显式绑定（插件 id、精确版本、标量 Settings）。省略或空列表表示
没有插件 MCP，从不自动发现。Settings 只在内存和成功的 ACP `session/new` 中，不进入 ledger 或 history。
上限、能力门禁、错误码和包隔离见 [Session MCP](../../docs/agents/session-mcp.zh.md#node-会话)。

`session_host_mcp_servers` 声明支持 `AgentSessionSpec.host_mcp_servers`：Controller 提供的 HTTP Server，在每次
ACP setup 中附加在插件 Server 之后；URL 与请求头只在内存中，ledger 只保留名称。见
[宿主 MCP Server](../../docs/agents/session-mcp.zh.md#宿主-mcp-server)。
