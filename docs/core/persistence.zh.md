# 持久化

[English](persistence.md) | 中文

Desktop 把应用状态保存在一个由 `ora-db` 拥有的 SQLite 数据库里。`ora-db` 是唯一知道 Ora SQL 的 crate：它定义迁移目录、启动数据库，并实现 `ora-application` 和 `ora-effect` 声明的仓储端口。上层调用方永远看不到行、列或软删除的记账字段。Node 和进程宿主各有独立的数据库，见 [Node](../node/node.zh.md) 和[进程宿主](../process/host.zh.md)。

## 迁移目录

迁移是 `crates/db/src/migration` 里的 Rust 代码，而不是独立的 `.sql` 文件；目录是 schema 的唯一事实来源，仓库里没有 `schema.sql`。

- 每个迁移有唯一、严格递增的版本号（`0001`），以及有序的 `up` 和 `down` 语句列表。去掉首尾空白并拼接后的 SQL 就是执行后持久化的快照。
- `MigrationCatalog` 在构建时校验这些不变量，所以重复或乱序的版本会在执行任何语句之前失败。
- `migrations` 记账表保存 `version`、`up_sql`、`down_sql` 和 `executed_at`。
- 每个方向的迁移及其记账更新都在同一个 SQLite 事务里完成。
- 目录在版本化历史之外还可以带一份首次安装 SQL。公开构建中它为空；发行版可以填充它而无需改写迁移。
- 已发布的迁移只追加不修改。如果别的分支先占用了编号，后合并的迁移要在合并前顺延编号；启动时会按精确的 SQL 识别一个历史情况（一个未发布的 Loop 分支曾使用 `0011`），并把它采纳为 `0013`。

| 版本   | 新增                                                                                                |
| ------ | --------------------------------------------------------------------------------------------------- |
| `0001` | 用户配置、项目、Workspace 位置与 provisioning、Workspace、worktree、任务标签、归属 Workspace 的会话 |
| `0002` | 带命名空间的 Skill 和可配置 Agent                                                                   |
| `0003` | 工作流定义、快照、归属 Workspace 的运行和节点运行                                                   |
| `0004` | 持久化的 Git 清理任务和 worktree provisioning 租约                                                  |
| `0005` | 插件 marketplace 源配置                                                                             |
| `0006` | 通用 Effect 持久化，见 [Effect](../agents/effects.zh.md)                                            |
| `0007` | 每个 marketplace 源的不可变命名空间绑定，以规范 Git URL 为键                                        |
| `0008` | 每个源的 `enabled` 标志                                                                             |
| `0009` | 每个源的制品获取配置（默认 Direct HTTPS）                                                           |
| `0010` | 独立的 Effect 恢复检测时间；Scope 的创建移入 Workspace 事务                                         |
| `0011` | 每个会话持久化的 MCP 选择（已有会话得到空的显式集合）                                               |
| `0012` | 节点运行上的 Iteration 轮次索引                                                                     |
| `0013` | 工作流执行作用域：根执行与 Loop 轮次的身份、作用域内的节点唯一性、降级归档                          |
| `0014` | 扩展包安装归属（`pack_installation`、`pack_installation_member`）                                   |
| `0015` | 定时任务、定时运行和触发点                                                                          |
| `0016` | 定时任务的调度锚点和 worktree 基础分支                                                              |
| `0017` | 触发时选定的定时运行 worktree 基础分支                                                              |
| `0018` | 定时运行关联的 worktree 任务                                                                        |
| `0019` | `finalized_without_effect` 的 Effect 操作阶段，通过暂存副本重建操作日志引入                         |
| `0020` | 定时运行的业务结论、明细与注解时间                                                                  |
| `0021` | 定时运行的产物清单与被拒绝候选记录                                                                  |
| `0022` | 释放卡在 `provisioning` 的本地 Main Workspace（仅数据；降级为空操作）                               |

单个迁移的细节见 [`crates/db/src/migration/README.md`](../../crates/db/src/migration/README.md)。目录只描述当前 schema；`migrations` 表早于 SQL 快照机制的数据库不受支持，需要重建。

## 应用启动

`DatabaseBootstrapper` 检查数据库和目标是否有完全相同的版本前缀，按升序补上缺失的尾部；如果数据库里有更新版本应用引入的尾部，就用那个新版本存下的 `down_sql` 逆序回滚。它从不比较共同版本的 SQL，所以打包后的应用不会因为旧迁移的定义变了就重建用户数据。

如果数据库原本没有任何迁移记录，并且至少应用了一个迁移，首次安装 SQL 会在之后单独的事务里执行，并且不记录到 `migrations`。已有的数据库永远不会执行它。如果它失败，启动失败，但已提交的迁移保持生效。

## 开发期协调

`reconcile_migration_history` 是开发期处理 SQL 漂移的显式工具接口。目录携带完整的迁移列表和一个活动目标，目标必须是列表的前缀，这样历史保持线性，回滚是确定的。

- 先校验共同前缀里的版本；未知或位置不对的版本是硬错误。超出目标的已应用尾部是回滚输入，不要求存在于目录中。
- 从前缀开头逐个比较持久化的和当前的 `up_sql`/`down_sql`（忽略 `executed_at`）。在第一个不一致处，按逆序回滚该迁移以及之后已应用的全部迁移，回滚始终使用数据库里存的 `down_sql`，然后应用当前的尾部并记录新的快照。
- 内容一致但缺版本时只应用尾部；目标更短时只回滚。已经一致的数据库不做任何事。
- 如果改写后的 `up` 失败，已经完成的回滚步骤保持已提交。

`cargo xtask reconcile-migrations DATA_DIRECTORY` 调用这个接口，`task run:desktop` 在启动 Tauri 之前对仓库的 `.data` 目录执行它，因此回滚和改写支持只存在于本地开发中。

## 连接

`DatabaseBootstrapper::bootstrap_repository_pool` 协调 schema 后返回可克隆的 `RepositoryPool`（基于 `rusqlite` 的 `r2d2`）。每个连接都由连接管理器统一配置，任何仓储都不可能以不同的配置运行：

- `journal_mode = WAL`
- `busy_timeout = 5000` 毫秒
- `synchronous = FULL`

断电持久性需要 `FULL`：WAL 模式下 `NORMAL` 追加 COMMIT 帧时不做 fsync，一次断电可能让数据库回退掉整整一次会话的写入。`FULL` 在每次提交时对 WAL 做 fsync，所以最多丢失正在进行的那个事务。Skill 包的提升会在提交之前刷新目录元数据，因此丢失最后一个事务只会留下一个无主的包目录，由启动协调删除，而不会出现有行无文件的情况。

连接池要求文件型的 `DatabaseLocation::Path`；`DatabaseLocation::InMemory` 用于启动和迁移测试。父目录由 Desktop 组装根准备好。

## 仓储

| 适配器                                                                                                                 | 端口或角色                                                                       |
| ---------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `SqliteProjectRepository`、`SqliteTaskRepository`、`SqliteWorkspaceRepository`、`SqliteWorktreeRepository`             | 项目、任务、Workspace 和 worktree 记录                                           |
| `SqliteTaskWorkspaceRepository`                                                                                        | 新任务的单事务提交，见[任务与 worktree](../workspaces/tasks-and-worktrees.zh.md) |
| `SqliteWorktreeProvisioningLeaseRepository`、`SqliteGitCleanupJobRepository`                                           | provisioning 租约和持久化的 Git 清理队列                                         |
| `SqliteCascadeRepository`                                                                                              | `ora-backend` 使用的聚合删除                                                     |
| `SqliteSessionRepository`                                                                                              | 会话                                                                             |
| `SqliteSkillRepository`、`SqliteAgentDefinitionRepository`                                                             | Skill 和 Agent 目录                                                              |
| `SqliteWorkflowRepository`、`SqliteWorkflowRunRepository`、`SqliteWorkflowRunEngineRepository`                         | 工作流定义、运行 CRUD 和运行引擎的事务                                           |
| `SqliteScheduledTaskRepository`、`SqliteScheduledRunRepository`                                                        | 定时任务及其运行                                                                 |
| `SqlitePluginMarketplaceSourceRepository`、`SqlitePluginSourceNamespaceRepository`、`SqlitePackInstallationRepository` | marketplace 源、命名空间绑定和扩展包归属                                         |
| `SqliteEffectRepository`                                                                                               | Effect 状态，见 [Effect](../agents/effects.zh.md)                                |
| `SqliteUserConfigRepository`                                                                                           | 用户配置的通用键值存储                                                           |

新增适配器从不改变端口签名。

### 可见性与软删除

`is_deleted = 1` 从不离开这个 crate。`find_*` 对软删除的行返回 `None`，`list_*` 只返回可见行，`soft_delete_*` 设置标志、刷新 `updated_at`，并报告是否影响了一个可见实体，handler 据此区分删除成功和未找到。

会话的修改不是整体快照替换：`update_session_title`、`update_session_status`、`update_session_binding` 和 `update_session_history_state` 各自只更新该业务意图拥有的列，并用 `RETURNING` 返回最新的完整行，所以持有旧快照的 actor 不会覆盖无关的修改。

### 行映射

仓储把列映射到当前的领域形状，包括枚举型的列。无法识别的持久化类别是映射失败，绝不会被默默转换成默认值。`worktrees.workspace_id` 既是主键也是所属 Workspace 的外键，所以 worktree 不可能拥有独立于其 Workspace 的身份。

`user_config` 适配器实现来自 `ora-user-config` 的通用原始键值端口，只负责读取、upsert 和删除。类型化的解释在它之上：应用层负责开发者模式、日志级别和网络代理的默认值与校验，Backend 负责 `worktree_root` 的路径策略。格式错误的持久化值会显式失败，每次写入只更新请求的那个 key。

### 聚合删除

`SqliteCascadeRepository` 在一个 immediate 事务里软删除整个任务或项目聚合，并在写锁下重新检查存在性和运行中的下级，所以会话不可能在校验和更新之间变成 `Running`。级联会读取每个有 worktree 的任务的 Git 身份（仓库根、分支、记录的 checkout 路径），在同一个事务里插入 `git_cleanup_jobs` 行；工作流运行的删除也会登记同样的任务。结果是 `CascadeDeleteOutcome`（`Deleted`、`NotFound` 或 `ActiveSession`）；`ora-backend` 把 `ActiveSession` 映射为 `resource_in_use`。这些事务从不调用 Git，也从不删除 provider 拥有的 ACP 历史。

### 错误与时间

SQLite 的执行、查询和映射失败被包装进应用层拥有的 `RepositoryError`，它把具体的 `DatabaseError` 作为 `Error::source()` 保留，而不是转成字符串。启动和迁移失败以 `DatabaseError` 形式出现。

迁移记账的时间戳来自注入的 `TimestampSource`。实体的 `created_at` 和 `updated_at` 来自应用层的 `Clock`。Effect 仓储在获取写事务之后从自己的 `TimestampSource` 取样，见 [Effect](../agents/effects.zh.md#审计时间与业务时间)。

## 运维日志

启动和协调会发出结构化事件：打开和启动事件带有 `operation`（`database_open`、`database_bootstrap`），报告中包含已应用、目标和待处理的数量，每个迁移步骤带有 `migration_version` 和 `direction`。失败在返回原始 `DatabaseError` 之前以 `ERROR` 记录。见[日志](logging.zh.md)。
