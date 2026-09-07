# 集成层重构验收

执行日期：2026-09-06（Asia/Shanghai）。历史基线为 `65ebdea`；正式增删演练基于
`1723a13904b5f5faedc16b6cbb1639e6ab8d9cc0`。按 Eric 指示跳过根决策检查，本记录不改变
`specs/` 的决策状态。完整行为证据见 [进度记录](refactor-progress.md)。

## 实际增删演练

在独立 detached worktree 中依次新增、验证、删除 unary 和 stream，两次之间恢复干净状态。
用 `deno install --frozen` 建立独立 workspace 依赖，确认 `@ora/contracts` 解析到临时工作树，
而非主工作树源码。Cargo 仅共享构建缓存。临时功能与工作树最终均已清理，未动用户数据。

| 场景                                     | 手写实现/声明文件 | 生成文件 | 测试文件 | 总变更文件 |
| ---------------------------------------- | ----------------: | -------: | -------: | ---------: |
| Settings 新增 `probeDeveloperMode` unary |                 5 |        6 |        2 |         13 |
| Files 新增 `watchWorkspaceProbe` stream  |                 4 |        5 |        2 |         11 |

两者都只维护四类手写事实：**契约、逻辑 operation、Desktop binding、所属业务实现**。
Unary 的业务实现包含 Settings handle 和 Desktop adapter 两个文件，因此目标不是“必须只改四个
文件”。生成产物、测试和文档独立计数；这也解释了为什么不能直接用 PR #508 的 50 个文件判断耦合。

| 事实                      | Unary 手写位置                                                     | Stream 手写位置                                  |
| ------------------------- | ------------------------------------------------------------------ | ------------------------------------------------ |
| DTO 与所属 family export  | `crates/contracts/src/developer_mode.rs`                           | `crates/contracts/src/file_system.rs`            |
| namespace/member/DTO/mode | `xtask/src/frontend/namespaces/developer_mode.rs`                  | `xtask/src/frontend/namespaces/file_system.rs`   |
| handler 与 host 授权关系  | `apps/desktop/src-tauri/bindings/developer_mode.rs`                | `apps/desktop/src-tauri/bindings/file_system.rs` |
| 所属实现                  | `crates/backend/src/settings.rs` 与 Desktop `commands/settings.rs` | Desktop `commands/files.rs`                      |

Unary 接收 nonce，经 Settings 读取真实存储偏好并返回关联响应，Desktop 使用现有 request lifecycle。
Stream 用新请求 DTO 定位 task checkout，创建原生 watcher 并交给现有 StreamStart，复用文件事件 batch。
两者都未修改根 Backend、通用 command helpers、stream startup/forwarding/registry、
`tauri-transport.ts`、client 执行机制或两份 Webview capability JSON。

Unary 自动更新 client、DTO、endpoints、Desktop map、command registry 和主权限文件；Stream
自动更新 client、DTO、endpoints、Desktop map 和 typed stream routes。没有手改生成文件。

| 检查点                 | Unary operation | Stream operation | 主 Webview command grants |
| ---------------------- | --------------: | ---------------: | ------------------------: |
| 干净基线               |             105 |                5 |                       130 |
| 新增 unary             |             106 |                5 |                       131 |
| 删除 unary 并重新生成  |             105 |                5 |                       130 |
| 新增 stream            |             105 |                6 |                       130 |
| 删除 stream 并重新生成 |             105 |                5 |                       130 |

Stream 复用 `stream_contract` / `cancel_contract_stream`，不增加共享机制或授权入口。
插件 Webview 仍只有原 invoke bridge，未因为新增 handler 扩大能力。

## 验证结果与复现

两次新增都执行 `task export-contracts`、`task check:contracts`，再次生成并对比完整 tracked diff。
两轮结果一致；生成检查另外按文件所有权核对产物，不能仅靠 tracked diff 排除未知/失效文件。
本次 tracked diff 的 SHA-256（重复生成核对值，不是发布签名）：

- Unary：`662be1f8bcb675e12b8c790c1d49437a8545eff4b79dbcc339e100b926123666`。
- Stream：`a6d27c978630991812d1f4da4cc87abe866dfba0bd5e725c112c9fe7b81997c9`。

Unary：真实 SQLite Settings interface 测试通过；contracts 10 项测试通过，包含真实 generated
client 的新 DTO、完整请求和 AbortSignal options；Desktop app/node 类型检查、ESLint，以及
`task test:tauri` 的 lint 和 58 项测试通过。

Stream：contracts 10 项测试通过，包含 DTO/options、惰性消费和 iterator return 的 finally 清理；
Desktop 类型/lint 与 42 项 transport/platform 测试通过，新增场景经真实 client/Tauri transport
验证分类、首帧、stream id 和公共 cancel command；58 项 Tauri 测试与 Rust 规模检查通过。
原生 watcher/转发沿用既有测试，新增 mock-invoke 测试不被当作真实 Webview 到原生文件系统的
端到端证据。

删除时先只移除所属手写声明、实现和临时测试，再运行生成检查：unary 按预期报出 6 个残留产物，
stream 报出 5 个残留产物，均明确失败。分别重新生成后，检查通过、contracts 恢复为 9 项测试通过，
`git status --porcelain=v1` 为空；stream 删除后 Desktop 类型/lint 也通过，数量恢复到上表基线。

按 [feature 增删指南](feature-change-guide.md) 在独立工作树复现上述两个场景。每次删除后都要求
重新生成、检查通过且工作树干净；保留验收证据，不把临时产品 operation 合入主 catalog。

## 验收中修正的遗漏

首次额外运行 Desktop TypeScript 检查时，发现阶段 2 取消测试使用 ES2024 的
`Promise.withResolvers`，而仓库目标是 ES2023。原 Desktop lint 只有 ESLint，之前全量测试未覆盖
此类型入口。`1723a13` 改为等价的显式 Promise，并把 app/node 类型检查接入 Desktop lint；正式
演练从修正后的干净基线重新执行，没有用提高 lib 目标的方式绕过。

## 最终主工作树验收

2026-09-06 最终 `task test` 全量通过，包含新增 Desktop 类型门禁、catalog/生成/授权检查、
feature interface 检查、Rust 规模门禁、tooling 测试、全部 frontend 和 Rust workspace lint/测试、
58 项 Tauri 和 7 项 E2E。app-shell 为 143 个文件、1285 项 clean-stderr 测试；Backend 220 项通过，
另有 1 项原有外部发布产物测试 ignored，未把缺失 live plugin-home fixture 的条件场景当作完整验证。

最终源码已无根 Backend 业务转发、中央 query-key 工厂、全领域 mock client 或直接替换 generated
client 方法的测试注入。翻译、共享缓存、typed 测试 adapter 和 public feature interface 已有明确
归属与检查。Files 保留显式 Explorer/Search 组合，未引入通用 view/plugin 容器。

7 个既有超限 Rust 模块仍是后续债务，并非本次全部拆完；owning crate、精确基线和拆分方向见
[架构检查说明](architecture-checks.md)。门禁禁止新增超限、现有超限增长或保留过时例外。
本方案未要求改变的 Effect generation、plugin identity、持久化布局和运行时资源所有权保持不变。
