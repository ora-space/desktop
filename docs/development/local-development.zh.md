# 本地开发

[English](local-development.md) | 中文

## 前提

安装 `.deno-version` 固定的 Deno 版本、Rust（见 `rust-toolchain.toml`）、Task、Git、ripgrep，以及所在平台的 Tauri 系统依赖。
不需要 Node.js 和 pnpm。

```sh
task install:frontend   # 通过 Deno 安装锁定的 npm 依赖，并安装 Git hook
task run:desktop        # 以开发模式运行 Desktop
```

`task --list` 是任务的权威列表。

## 工具链固定

`.deno-version` 是 CI 和 sidecar 安装使用的 Deno 版本；要让 `package.json` 的 `engines.deno` 与它保持一致
（`deno task check:toolchain` 同时检查已安装的运行时和声明，并在安装、工具检查和打包之前运行）。`DENO_VERSION` 只能重复这个共享
的版本。

根目录的 `package.json` 拥有 workspace 的 TypeScript 版本；每个包都使用它的 `tsc` 和编译器 API，子包不固定自己的编译器。
`deno check` 使用固定版本 Deno 自带的编译器。`ts-to-zod` 这类第三方工具保留它们依赖的 TypeScript 5 API。

## 运行 Desktop

`task run:desktop` 构建 debug 版的 `ora-reaper` sidecar，对仓库的 `.data` 目录运行 `cargo xtask reconcile-migrations`（见
[持久化](../core/persistence.zh.md#开发期协调)），然后在 `ORA_DATA_DIR` 指向 `.data` 的情况下启动 Tauri。该数据库中相对的项目
位置相对于仓库根目录解析。debug 构建使用 `PATH` 中的 `deno` 和 `rg`。

- 日志在 `.data/logs/ora.log`；`task logdy` 在本地的 Logdy UI 中跟踪它们。
- Agent 来自插件，所以开始聊天之前，先从市场源安装 Agent 插件，或导入一个本地的 `.orax`。
- `task package:desktop` 按 release 工作流的方式构建完整的本地安装包，并启用 release 版的 sidecar。

## Controller 和 Node

Controller 和 Node 只支持 Linux：

```sh
cargo build -p ora-controller -p ora-node -p ora-reaper
```

它们的配置见 [Controller](../controller/controller.zh.md) 和 [Node](../node/node.zh.md)。Cloud 契约的 submodule 用
`task proto:init` 初始化，见 [Cloud 契约](../controller/cloud-contract.zh.md#获取与生成契约)。连同 Cloud、Gateway 和真实沙盒的
端到端运行使用 `ora-space/cluster` 中的 Compose 环境，它会构建[容器镜像](../operations/container-images.zh.md)。

## 维护脚本

- `deno run -A scripts/repair_workflow_snapshots.mjs --database <absolute-path-to-ora.sqlite3>` 预览对符合条件的工作流草稿的
  布局修复；在完整备份 SQLite 之后，`--apply` 应用这些修复，见[工作流](../workflows/workflows.zh.md#在编辑器中加载)。
  `deno task test:tooling` 运行它基于临时数据库的回归测试。
- `deno run -A scripts/workflow-qualification-suite/validate_sample.mjs` 通过运行中 Desktop 的 `analyzeWorkflow` 分析所有生成的
  资格验证图，不创建工作流。执行运行器要求 `ORA_QUALIFICATION_WORKSPACE_ID` 指向一个活跃的、可丢弃的隔离 Workspace，因为 Agent
  可能会编辑其中的文件。报告陈述实际测得的 Backend 结果，并显式列出未测量的 UI 和内存检查；失败时以非零状态退出，报告只留在本地。
