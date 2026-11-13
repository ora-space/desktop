# 测试

[English](testing.md) | 中文

测试的编码和断言规则在仓库的 `AGENTS.md` 中；本文列出各个检查关口以及它们覆盖的内容。

## 任务分组

| 任务 | 覆盖 |
|---|---|
| `task test:frontend` | 先 `check:contracts`，再对每个前端包做 lint 和测试，包括 Desktop 的 TypeScript 传输层 |
| `task test:crates` | 可复用的 Rust crate |
| `task test:tauri` | Tauri Desktop 包 |
| `task test:e2e` | Desktop 端到端测试（`e2e/`，使用假的 ACP Agent） |
| `task test` | 以上全部再加所有 lint；耗时较长 |
| `task lint`、`lint:frontend`、`lint:crates`、`lint:tauri`、`lint:e2e` | 格式化、Clippy 和各类 linter |
| `task check:contracts` | 生成的契约和插件协议绑定与它们的来源一致 |
| `task check:features` | 功能接口的可见性，见[前端架构](../core/frontend-architecture.zh.md#公开的功能接口) |
| `task check:rust-size` | 生产 Rust 模块的大小，带只能收紧的例外列表（`xtask/rust-size-baseline.json`） |
| `task proto:check` | Cloud 契约 submodule 固定在指定 commit，生成的客户端与之一致（属于 `lint:crates`） |
| `deno task test:tooling` | `scripts/` 下的工具脚本 |

Tauri crate 与其他 Rust crate 共享根目录的 `Cargo.lock`、依赖图和 target 目录。CI 分别运行各组，crates 任务在 Linux、macOS 和
Windows 上运行。迭代时使用最小的相关任务；Desktop 的改动需要 `task test:tauri`，不能只跑 workspace 测试，横跨多处的改动最后要跑
`task test`。

## 前端

前端测试在 `scripts/run-with-clean-stderr.ts` 下运行：stderr 上的任何警告，尤其是 act() 警告，即使 Vitest 显示通过也会让整次运行
失败。测试使用生产生成的客户端加带类型的操作处理函数（见[前端架构](../core/frontend-architecture.zh.md#带类型的测试传输层)），
渲染翻译时自己 import `appI18n`，并在真实的异步边界上 await 每个可能更新 React 的操作。contracts、chat 和 editor 包先运行
`tsc --noEmit`，再运行 `deno test --no-check`。

## Rust

当 SQLite 和 Git 正是被测行为时，测试优先使用带真实 SQLite 和真实 Git 的生产接口，注入时钟和依赖而不是修改进程环境，并用
`pretty_assertions::assert_eq` 比较整个对象。发出 `tracing` 事件的代码在线程范围的 TRACE 辅助函数下运行，见
[日志](../core/logging.zh.md#测试)。

有些测试套件需要额外准备：

- Node 独立测试运行真实的 Node、Controller 和 reaper 可执行文件；先用
  `cargo build -p ora-node -p ora-controller -p ora-reaper` 构建它们。
- clone、交付、恢复和 `ora-git` 的测试运行一个本地 Git Server 替身，它要求客户端证书 TLS、由 `git http-backend` 支撑（`ora-test-support` 的 `git-server` feature），只在 Linux 上构建。
