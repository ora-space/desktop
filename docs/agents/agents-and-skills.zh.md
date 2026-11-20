# Agent 定义与 Skill

[English](agents-and-skills.md) | 中文

除了插件提供的 Agent 运行时，Desktop 还维护两个用户目录：**Agent 定义**（工作流节点或聊天可以采用的可配置角色）和 **Skill**（物化到每个 Workspace 的可复用指令包）。两者都以 `(namespace, name)` 区分，可见名称不区分大小写；本地条目使用 `local` 命名空间，软删除的行不占用名称。

## Agent 定义

`AgentDefinition` 由带命名空间的名称、描述和 Markdown 内容组成，存在 `agents` 表中。`agent` 操作是普通的增删改查（`createAgent`、`getAgent`、`listAgents`、`updateAgent`、`deleteAgent`）；删除是软删除。定义只是配置记录：它们不发现模型、不启动 CLI，也不拥有会话生命周期。设置界面把它们呈现为"角色"。工作流 Agent 节点引用它们，创建运行时会把解析出的角色冻结进运行，见[工作流运行](../workflows/workflow-runs.zh.md)。

`prepareAgentImport` 和 `commitAgentImport` 把一个 Markdown 格式的 Agent 文件（最多 1 MiB，YAML front matter 含 `name` 和 `description`）导入 `local` 命名空间。重名时提交需要显式的 `skip` 或 `overwrite` 决定，以及准备阶段得到的冲突行的 id 和 `updatedAt`；如果那一行在此期间变了，结果是 `stale_conflict`，不写入任何内容。

## Skill

Skill 是一条目录行加上磁盘上的包 `<数据根目录>/atoms/skills/<name>/`，包的根目录有带 YAML front matter 的 `SKILL.md`。`Skill.source` 是 `local` 或 `plugin { pluginId }`：插件可以附带 Skill，它们会连同来源一起导入目录。名称是不以 `.` 开头的单个 ASCII slug 片段，描述最多 4096 字节。

已安装的 Skill 插件可以为脚本声明公开与敏感字段。声明方式、启动环境快照、重启生效和安全边界见 [Skill 插件配置](skill-configuration.zh.md)。

- `availability` 根据包计算：目录缺失或 `SKILL.md` 无法加载时，Skill 标为不可用，而不是被删除或让请求失败。新建或导入同名 Skill 会恢复那一行的包并保留它的 id。
- 新建时同时写入行和一个最小的 `SKILL.md`。更新时先暂存一份副本，只改写 manifest（保留未知的 front matter 和正文），名称变化时重命名目录，并保留未修改的包文件。删除时软删除行，并把目录移入事务备份。

### 包的原子性

SQLite 与文件系统之间的原子性来自日志和启动恢复，而不是一个跨越两者的事务。每次变更都先把包暂存到保留目录 `<Skill 根目录>/.ora-staging`，在 `.ora-journal` 下写一个 `Prepared` 标记，提升正式目录，把日志标为 `Swapped`，写入行，最后才清除日志和备份（`.ora-backup`）。行写入失败会回滚目录。重命名期间从不持有 SQLite 事务，所以一个缓慢的 Skill 根目录不会饿死其他写入方。

包文件、日志标记和目录提升都在行提交之前刷盘，SQLite 又以 `synchronous=FULL` 运行，所以唯一剩下的崩溃窗口是"目录已存在、行尚未提交"，由启动协调删除无主的包来修复。变更时的目录 fsync 在 Unix 上是硬错误，在 Windows 上是尽力而为。启动会等这次协调完成后才开始服务请求。

细节见 [`crates/application/src/skill/README.md`](../../crates/application/src/skill/README.md)。

### 导入 Skill

Skill 导入是一个两阶段会话，来源是一个文件夹树或一个归档（`.zip`、`.skill`、`.tar.gz`、`.tgz`）：

1. `prepareSkillImport` 接收系统选择器选中的本地路径（由 Desktop 读取文件，所以最多 200 MiB 的内容从不经过 IPC），在操作系统临时目录中物化一个经过校验的快照，找出互不重叠的 `SKILL.md` 边界并解析每个 manifest。它从不改动正式存储。
2. `getSkillImport` 预览每个候选：`ready`、`conflict`（已有包可用的 Skill，提供 `skip` 或 `overwrite`）或 `invalid`。
3. `commitSkillImport` 要求每个冲突都有决定，冻结这些决定后按稳定的路径顺序在后台导入。每个 Skill 与它的行一起原子地提升，一个失败从不回滚其他 Skill。已接受的提交不能取消；用相同的决定重试会回放已存的结果。
4. `cancelSkillImport` 结束尚未提交的会话。

会话只存在于内存中并会过期；已完成的会话在保留期内保存轻量的结果。归档安全（zip-slip、加密和特殊条目、可移植的大小写冲突、条目和字节预算）由 `ora-utils::archive` 和 `ora-utils::path` 负责；Skill 的上限和 manifest 解析由 `ora-skill-package` 负责。插件 surface 可以从一次下载触发同样的导入，见 [Surface](../plugins/surfaces.zh.md#下载)。

## Skill 如何到达 Agent

已安装的 Skill 由 Effect 系统物化到每个符合条件的 Workspace，见 [Effect](effects.zh.md)。工作流 Agent 节点的 Skill 绑定表示在该节点的 prompt 里"必须调用"；它们不是节点级的允许列表，也不会隐藏其他已物化的 Skill。
