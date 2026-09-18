# Controller–Node 校验证据

[English](controller-node-validation.md) | 中文

公开 codec 集成测试从 `crates/node-protocol/tests/protocol.rs` 进入。`protocol/session.rs`、
`worktree.rs`、`execution.rs` 分别拥有业务 fixture 和约束；`support.rs` 共享 framing 与断言，
`framing.rs` 拥有不依赖具体 Transport 的帧测试。较大的 Worktree 和 execution 字段表位于各自
的 `rejections.rs` 子模块。接收反例使用独立 JSON 和手工 framing，不经过公共 writer。
语义反例还反序列化为无效 typed message 后调用 writer，检查相同的精确错误和空输出。

## Clone 结果扩展（2026-09-18）

当前共 36 项测试。`repository.rs` 拥有 clone 请求测试，`repository_results.rs` 拥有成功／失败
样例及其 `clone_result` 事件、Completed 查询两种形态，包含保留残留和无自有目录的失败。
覆盖精确 wire／分片 I/O、输入与结果 Node 冲突、历史实例、目标／commit 校验、原始诊断拒绝及
与 Worktree 不重叠的标签。`session::accepts_independent_and_combined_execution_capabilities`
覆盖 clone-only、组合声明以及重复／未知能力拒绝。

Worktree Completed 样例和字段表移到 `worktree/completed/`；`execution/` 只保留通用命令及
非终态结构测试，两种业务各自调用共享的历史报告者断言。Worktree 持久编码及数据库 schema 不变。
这些测试不证明 clone 执行、会话派发能力检查或 Controller 接管。

## 证据及原 17 个测试的迁移

| 保证／原覆盖                                                             | 当前证据                                                                                                                                    | 状态                  |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- |
| 两个全局 round-trip 测试                                                 | `session::round_trips_messages`、`worktree::round_trips_messages`、`execution::round_trips_messages`；通过三字节 duplex buffer 比较完整消息 | 已覆盖                |
| 仅 Hello 的固定信封样例                                                  | 各业务的 `preserves_wire_shapes`：独立 JSON 与 writer 输出相等，手工 framing JSON 的 reader 输出与完整 typed message 相等                   | 已扩展                |
| 干净 EOF、截断头／payload、零／超长长度、未知类型、非法 JSON 和超长发送  | `framing.rs` 中全部八个测试，含超长发送零写入                                                                                               | 已保留                |
| 全局缺失／空字符串扫描                                                   | 各业务的 `rejects_missing_and_empty_fields`，分别显式列出必填路径和非空字符串                                                               | 已迁移，不推断 schema |
| 全局版本和握手矩阵                                                       | 各业务的 `rejects_versions_directions_and_payloads`；`session::rejects_inconsistent_handshakes` 使用具名 Hello 和 HelloAccepted fixture     | 已保留                |
| 错方向、不匹配 payload、缺失 type/version/payload/sequence               | 业务信封检查及必填字段表覆盖每种消息形状的双向限制                                                                                          | 已保留                |
| 未知 state、Completed 缺失 result、非终态附带 result、不完整终态 payload | `execution::rejects_structural_state_contradictions`                                                                                        | 已保留                |
| 四种 Completed 保留历史实例且拒绝其他 Node                               | `worktree::completed::preserves_completed_worktree_contracts`：独立 wire 与 typed fixture、分片往返、精确收发错误及零写入                   | 已保留                |
| 透明的 operation identity                                                | `execution::serializes_identity_consistently_across_messages`                                                                               | 已保留                |
| 接受的 opaque 字符串及未知扩展字段                                       | 字段测试给所有 opaque 字符串加空白后比较完整 wire/typed message；wire 测试添加未知信封及 payload 字段后比较解码消息                         | 新增直接证据          |

### 字段覆盖清单

下表逐项标明原全局 fixture 及其全部 opaque 字段的新归属。每个必填表还包含 `message_type`、
`protocol_version` 和 `payload`。缺失字段产生 `DecodeJson`；空字符串和全空白在收发两端
产生精确 `EmptyField`。每个基准消息先验证合法；替换指针必须存在，删除必须实际移除字段。

| 业务 fixture                                                 | 信封之外的必填字段                                                                                                                                      | 非空值／特殊规则                                                             |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Session Hello                                                | `controller_id`、`supported_versions`                                                                                                                   | Controller 身份；空列表、重复版本、未声明信封版本                            |
| Session HelloAccepted                                        | `selected_version`、Node 双身份、`capabilities`                                                                                                         | Node 双身份；selected/envelope 不一致、缺失／重复能力                        |
| Session Heartbeat                                            | Node 双身份                                                                                                                                             | NodeId 和 incarnation                                                        |
| Worktree Ensure / Remove                                     | operation/execution ID；spec NodeId、WorkspaceId、WorktreeId、repository、Main Workspace ID/path、base ref、expected branch、path-policy kind/directory | 所列字符串；可选 request ID 存在时必须非空                                   |
| Worktree Ready                                               | operation/execution ID、sequence；结果 Node 双身份、WorkspaceId、WorktreeId、facts path/branch/base commit                                              | 所列字符串及可选 request ID                                                  |
| Worktree Failed / RemovalFailed                              | operation/execution ID、sequence；结果 Node 双身份、WorkspaceId、WorktreeId、failure code/message                                                       | 除枚举 code 外的所列字符串及可选 request ID                                  |
| Worktree Removed                                             | operation/execution ID、sequence；结果 Node 双身份、WorkspaceId、WorktreeId、outcome                                                                    | 除枚举 outcome 外的所列字符串及可选 request ID                               |
| Execution GetStatus / EventAck                               | operation/execution ID、目标 NodeId；EventAck 还必需 sequence                                                                                           | 三个 ID                                                                      |
| Execution Unknown / Accepted / Running                       | operation/execution ID、报告者 Node 双身份、state 标签                                                                                                  | 四个 ID；非终态拒绝附带 result                                               |
| Execution Completed Ready / Failed / Removed / RemovalFailed | status 字段加 result kind 及上述对应 Worktree 结果的完整字段                                                                                            | 外层与嵌套的全部 opaque 字符串；内外持久 NodeId 必须相等，incarnation 可不同 |

固定 wire fixture 覆盖全部 12 种消息、六种 Worktree 消息有值及省略 `request_id`、
Unknown/Accepted/Running、包含四种终态结果的 Completed，以及 Removed/AlreadyAbsent
删除 outcome。预期 JSON 使用对象字面量，不序列化 spec 或 result 类型来生成；不约束对象
字段顺序。新增消息或字段时必须检视所属业务的 wire fixture 和显式字段表，字段表不会自动
发现新增 schema 字段。迁移核对确认 25 个合法 fixture 的 175 处 opaque 字段均与原扫描
覆盖一致，包含可选 request ID。

## 验证及边界

独立 wire 测试在未修改的生产实现上与原 17 个测试一起通过（该阶段共 20 个测试）。测试迁移
和生产重构后，24 个集成测试全部通过。`cargo test -p ora-node-protocol` 和
`cargo clippy -p ora-node-protocol --all-targets -- -D warnings` 通过。

仓库检查（2026-09-10）：`task format`、Rust workspace lint/测试、Tauri lint/测试和 Desktop
E2E lint/测试通过。首次 `task test` 停在 app-shell 的会话上下文菜单复制测试
（`chat-view.test.tsx:2235`，剪贴板 mock 未被调用）。该用例单独复跑通过，整个 app-shell
套件复跑的 1,317 个测试也全部通过。后续 plugin SDK 套件另行执行通过；因此 `task test`
所有组成检查均已执行，但首次组合调用失败。

测试验证消息合法性，不枚举全部非法 JSON。相同形状的创建和删除 payload 仍兼容。构造器和
直接 Serde 转换仍不执行语义校验。当前 typed value 没有会失败的自定义 serializer，因此未
诱发 JSON 序列化失败。零写入检查不证明任意 I/O 失败和部分写入后的原子回滚，也不宣称穷尽
mutation testing。

会话绑定、协商后的会话版本限制、派发授权、持久去重、副作用前持久化、确认前持久化、重放和
崩溃恢复仍由 Node、Controller、session 负责，这些测试不构成其实现证据。协议日志继续延期至
`todo-87602f0b`，源代码 TODO 覆盖另有文档记录。
