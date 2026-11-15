# ora-node

[English](README.en.md)

Rust Node 是执行程序。Cloud 网络入口要求双向 TLS、固定目标与 root 管理/UID 1000 工作负载分离；管理凭据和本地日志位于受保护路径。持久接受后、首次目录/Git 变更前分别检查资格，恢复不重新开放旧 incarnation（进程启动代次）。已有工作按原范围收尾。明文网络和全局个人 Git 凭据明确拒绝。examples/loopback-transport-fixture 仅用于真实 TLS、Git 和崩溃测试，使用注入的测试目标与绑定，不进入镜像。操作系统身份隔离由 cluster 真实沙盒验收。

## 验证与边界

模块测试覆盖真实协议、恢复日志、Git 或 TLS 的所属边界；cloud fixture 及回环程序不证明生产多人授权。真实部署用 cluster Compose，契约源为 third_party/cloud 固定提交 f0b5d9c，生成文件不手改。完整验收与缺口见根 [运行时控制说明](../../docs/controller/runtime-control.zh.md)。

## Session MCP

`session_mcp` 声明支持 `AgentSessionSpec.mcp` 显式绑定（插件 id、精确版本、标量 Settings）。省略或空列表表示
没有插件 MCP，从不自动发现。Settings 只在内存和成功的 ACP `session/new` 中，不进入 ledger 或 history。
上限、能力门禁、错误码和包隔离见 [Session MCP](../../docs/agents/session-mcp.zh.md#node-会话)。
