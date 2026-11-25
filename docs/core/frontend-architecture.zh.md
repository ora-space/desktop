# 前端架构

[English](frontend-architecture.md) | 中文

App Shell（`packages/app-shell`）按功能组织。本文说明横跨各功能的所有权规则：翻译、查询缓存、公开的功能接口和测试传输层。功能内部细节见各功能目录的 `README.md`。

## 翻译资源

功能文案放在纯数据的 `features/<feature>/translations.ts` 中；领域相关的契约错误放在该功能文案旁边。只有真正共享的外壳和传输消息留在 `i18n/common-resources.ts`。Settings 把通用设置、插件、Skill 和角色分别放在自己的条目里；工作流编辑器的文案归编辑器所有，即使历史上的 key 以 `settings.workflow` 开头；共享的剪切、复制、粘贴和全选文案归 `features/editor/translations.ts`。

`i18n/resources.ts` 显式组合这些条目。它不导入任何 React 或 i18next 代码，资源也从不导入其功能实现。`composeTranslationResources` 会拒绝重复的逻辑 key 所有权、缺失的语言 key 和不完整的复数组（英文 `_one`/`_other` 和中文不带后缀的计数消息按逻辑 key 比较）。

`i18n/i18n-instance.ts` 是 `appI18n` 唯一的初始化位置。它同步组合全部文案（`initAsync: false`），保留中文回退，以及 `ora.locale` 存储和文档语言行为。功能挂载时从不注册资源。任何渲染了 `useTranslation` 的测试都要自己导入 `appI18n`。

## 查询与失效的所有权

`state/data/` 负责缓存标识和共享数据规则。没有全局的 key 注册表或 barrel，使用方直接导入需要的 owner。

| Owner                                                                                | 负责                                                                    |
| ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| `workspace.ts`、`sessions.ts`                                                        | 共享列表、采纳权威响应、聚合删除后的清理、会话树中的位置                |
| `workflows.ts`、`workflow-analysis.ts`、`workflow-runs.ts`、`workflow-run-retry.ts`  | 定义、草稿、版本和执行分析查询；持久化运行的查询、变更、投影和重试展示  |
| `mock-workflows.ts`、`mock-workflow-runs.ts`、`mock-workflow-mounts.ts`              | 内存运行时的标识和订阅，与持久化运行缓存隔离                            |
| `files.ts`、`file-watch.ts`、`diff.ts`、`workspace-status.ts`、`workspace-review.ts` | 有作用域的文件访问、watcher 驱动的失效、Workspace diff、状态和评审刷新  |
| `plugins.ts`、`agent-runtime.ts`、`plugin-lifecycle.ts`、`mcp-health.ts`             | 目录和配置缓存、按 Agent 划分的模型前缀、跨领域的生命周期协调、MCP 健康 |
| `agents.ts`、`skills.ts`、`settings.ts`、`identity.ts`                               | 定义、偏好和身份缓存                                                    |

UI hook 负责选中状态、草稿和输入框清理、变更进行中状态以及视图相关的操作；它们先通过数据 owner 采纳权威结果，再协调 UI 状态。值得记住的规则：

- 删除项目时会清理 Main Workspace 和任务的会话，即使任务 Workspace 列表还没有加载。
- 列表改名会直接修补响应并以 `refetchType: "none"` 失效；单独删除会刷新活跃列表，而父级级联删除可以推迟子级刷新。不要用一刀切的失效替代这些规则。
- 应用事件 hook 负责重连和中止状态，但把缓存规则交给数据 owner。`Ready` 事件或断线会重新拉取会话，以弥补漏掉的事件；标题事件只做失效。
- 持久化运行详情的 key 是 `['workflowRun', 'detail', runId]`，内存运行时使用 `['workflowRun', runId]`。失效从不合并这两个空间。终态分类在 `@ora/workflow-runtime` 中。
- Files 负责显式的 Explorer/Search 组合，以及任务 worktree 与项目 checkout 之间的选择。改名事件会让新旧路径、它们的父目录和搜索失效；切换作用域会中止旧 watcher，卸载会中止当前 watcher。

插件刷新的范围是刻意划定的：

| 触发 | 失效 |
|---|---|
| Agent 激活 | 运行时可用性，以及该 Agent 在所有 Workspace 中的发现查询 |
| Agent 停止或移除 | 运行时可用性；不对已停止的运行时做发现 |
| 插件变更完成 | 已安装和可用插件，以及本次会话的 Hook 生命周期结果 |
| `PluginStatusChanged` | 已安装插件、运行时可用性和 Skill；不含可用插件和模型 |
| `AgentModelsInvalidated` | 只有该 Agent 的发现查询前缀，失效前先取消进行中的请求（当前暂无后端发送方） |
| 显式初始化 Hook | 只有 Hook 生命周期结果 |

数据测试使用真实的 `QueryClient` 缓存和 observer，验证精确的失效和重新拉取行为。

## 公开的功能接口

每个功能的 `interface.json` 记录其 owner，以及其他功能或外壳组合可以使用的具名导出，每一项都说明共享的原因。其余全部是私有的，包括同一文件里的其他导出。这是可见性策略，不是运行时注册表：调用方仍然直接具名导入，也不存在包含所有功能的 barrel。不要为了通过检查而列出所有导出或加通配。

`task check:features`（属于 `lint:frontend`）用 App Shell 的 TypeScript 配置解析导入，检查 `apps/` 和 `packages/` 中实际导入的符号：静态和动态导入、再导出、别名、内联类型导入、CommonJS 和 Vitest mock。对其他功能的命名空间导入或整模块访问会被拒绝，因为那会暴露私有导出；非字面量的模块访问同样被拒绝。测试可以用无参数的对象字面量工厂 mock 显式具名的公开 UI 导出；展开原模块不能作为绕过手段。

共享的 `state/` 代码不能导入功能 UI，即使那个 UI 是公开的。例如 Agent 目录在 `state/hooks/use-agent-catalog.ts`，评审尺寸在 `state/stores/review-layout.ts`，输入框引用操作在 `state/actions/add-composer-file-selection.ts`，工作流视口约束在 `workflow-node-chrome/viewport.ts`。功能翻译只能由 `i18n/resources.ts` 从 owner 之外使用。

## 带类型的测试传输层

`test/contracts-transport.ts` 提供 `createTestClient(handlers)`，它使用生产环境的生成客户端。`TestHandlers` 从操作目录派生每个请求、响应和 unary/stream 模式，因此 fixture 从不重复命名空间映射。传输层会拒绝遗漏或未知的操作以及模式不符；流在第一次消费时才查找 handler，迭代器退出会把清理转交给 handler。DTO、bigint 和 `AbortSignal` 选项原样穿过这一层。

`test/memory/` 负责各领域的状态、构造函数和操作 handler。测试只组合自己需要的领域：

```typescript
const state = createWorkspaceMemory();
const client = createTestClient(workspaceHandlers(state));
```

`emptyFilesHandlers` 这类合成 fixture 用名字说明自己的行为，从不作为全局默认；没有依赖的测试从 `createTestClient({})` 开始。自定义行为，包括脚本化的聊天流和注入的失败，都用带类型的 handler 实现。永远不要替换生成客户端的方法，也不要把部分对象强转成 `ContractsClient`。handler 断言要包含传输层的第二个参数 `options`。
