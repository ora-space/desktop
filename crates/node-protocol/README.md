# node-protocol

[English](README.en.md)

`ThreadEvent` 在原始定型记录之外可携带有界的 Agent 正文投影及截断标记。旧消息缺少此字段仍合法；它不代替原始记录、Thread 确认或终态。

Hello／HelloAccepted 的可选 `thread_previews` 字段协商实时旁路；未协商时不发送 `ThreadPreview`。该消息是有界的 execution/version 完整替换，没有持久事件 sequence 或 Ack。稳定历史位置由定型 `ThreadEvent.history_sequence` 关联，旧消息缺失时仍可解码。

RuntimeControl 能力代表用户控制约束；单连接和心跳不能替代它。RuntimeBinding 显式区分用户代次、Controller 代次、运行时代次、Node incarnation、业务 operation、Node operation 和 execution。Cloud 旧组件不支持时拒绝，不忽略字段。

## 未受理证明消息

Hello 的可选 `execution_non_acceptance` 标记启用同名能力协商；旧 Controller 不会收到新能力名，旧
Node 不声明能力时也不发送新命令。`FenceUnacceptedExecution` 携带原 work、冻结输入与实例身份，
`ExecutionNotAccepted` 仅携带持久未受理证明，没有普通事件 sequence 或 ACK。

## 验证与边界

模块测试覆盖真实协议、恢复日志、Git 或 TLS 的所属边界；cloud fixture 及回环程序不证明生产多人授权。真实部署用 cluster Compose，契约源为 third_party/cloud 固定提交 e48cc41，生成文件不手改。完整验收与缺口见根 [运行时控制说明](../../docs/controller/runtime-control.zh.md)。
