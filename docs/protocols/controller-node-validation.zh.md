# Controller–Node 校验证据

[English](controller-node-validation.md) | 中文

以下测试均使用 `crates/node-protocol/tests/protocol.rs` 中的公开 codec 及其
`protocol/` 子模块。无效的接收输入直接构造 frame，不经过公开 writer。评审基线包含 12 个测试；
Completed 修复增加 1 个测试，拒绝矩阵增加 4 个测试。

| 协议保证                                                                    | 基线证据                                                                                                                                  | 新增直接证据                                                                                                                                                                               | 当前状态 |
| --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------- |
| 分片 I/O 下每条消息都保留身份和事实                                         | `round_trips_every_controller_message`、`round_trips_every_node_message`                                                                  | 保留原测试                                                                                                                                                                                 | 已覆盖   |
| 正常 EOF 与截断 header／payload 有区别                                      | `returns_none_at_clean_eof`、`rejects_truncated_header`、`rejects_truncated_payload`                                                      | 保留原测试                                                                                                                                                                                 | 已覆盖   |
| 零长度／超大长度、未知 frame 类型、格式错误的 JSON 会被拒绝                 | `rejects_zero_length_frame`、`rejects_oversized_frame`、`rejects_unknown_frame_type`、`distinguishes_malformed_json_from_invalid_message` | `rejects_oversized_outbound_message_without_writing`                                                                                                                                       | 已覆盖   |
| 必需身份和不透明领域字段不能缺失、为空或全为空白                            | 仅直接测试了空 operation identity                                                                                                         | `rejects_missing_and_empty_fields`：覆盖每个 envelope、command spec、result 及嵌套 Completed variant；可选 request identity 可以缺失，但不能为空                                           | 已覆盖   |
| Envelope 版本和握手声明彼此一致                                             | 缺少直接的语义反例测试                                                                                                                    | `rejects_inconsistent_handshakes_and_versions`：覆盖每个 envelope 的版本、空／重复／未声明版本、选定版本不匹配、空／重复能力                                                               | 已覆盖   |
| 错误方向、不兼容 payload 结构、缺少 envelope 字段、非法执行状态组合会被拒绝 | `rejects_wrong_direction_message` 覆盖一个方向                                                                                            | `rejects_structural_contradictions`：覆盖两个方向的所有消息、无关 payload、缺少 type／version／payload／sequence、无 result 的 Completed、带 result 的非终态、未知状态和不完整终态 payload | 已覆盖   |
| Completed 属于报告 Node，同时保留历史 incarnation                           | 缺少                                                                                                                                      | `completed_results_preserve_incarnations_and_reject_other_nodes`：覆盖四种终态、整条消息相等性，以及跨 Node 的收发拒绝                                                                     | 已覆盖   |
| 发送端语义拒绝不会写入字节                                                  | 缺少                                                                                                                                      | `reject_semantics` 检查所有空字段和握手／版本场景的精确校验错误及空输出；Completed 和超大消息发送测试也检查空输出                                                                          | 已覆盖   |

当前 crate 有 17 个通过的集成测试。验证命令：`cargo test -p ora-node-protocol` 和
`cargo clippy -p ora-node-protocol --all-targets -- -D warnings`。

该矩阵覆盖当前消息合法性，不覆盖所有可能的格式错误 JSON 文档。Payload 兼容性表示满足所选
variant 的必需结构；结构相同的 create／remove payload 对任一消息都有效。未知扩展字段通常不被
禁止。测试没有主动制造 JSON 序列化失败：当前类型化值没有可能失败的自定义 serializer。任意
传输 I/O 失败以及部分写入后的原子回滚，均未被零写入校验直接证明。这里没有穷举式 mutation testing
结论。

会话绑定、协商会话版本约束、派发授权、持久去重、副作用前持久化、确认前持久化、重放和崩溃恢复，
仍属于后续 Node、Controller 及会话切片的职责。以上测试没有直接证明这些内容。协议日志仍延期至
`todo-87602f0b`；源代码 TODO 的覆盖情况另行记录。
