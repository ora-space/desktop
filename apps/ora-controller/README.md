# ora-controller

[English](README.en.md)

Cloud 模式只拨出到认证 gRPC，不持有 Cloud 业务 SQLite。每次新 clone/effect 派发重新取得 PostgreSQL 短期许可；关闭资格后只查询原执行。绑定确认、普通推进与独立强停分别恢复，所有结果保留稳定身份。

## 验证与边界

模块测试覆盖真实协议、恢复日志、Git 或 TLS 的所属边界；cloud fixture 及回环程序不证明生产多人授权。真实部署用 cluster Compose，契约源为 third_party/cloud 固定提交 e48cc41，生成文件不手改。完整验收与缺口见根 [运行时控制说明](../../docs/runtime-control.zh.md)。

会话派发、Thread 中继与命令投递见 [Agent 会话中继](../../docs/controller/agent-relay.zh.md)。
