# 仓库获取的持久责任

[English](repository-acquisition.md) | 中文

clone 在显式注入的原 Node 数据库中使用独立业务记录，不重新解释 Worktree 输入或终态。
Linux 运行时现已连接存储、原生目录身份、受管 Git 获取及重启协调；见[部署说明](../repository-clone.zh.md)。

`accept_clone` 原子保存原始命令、生成的仓库身份、根／目标路径及初始预留，不创建目录。
相同输入重放返回原目标；输入变化或跨能力复用 operation／execution 均失败。目标不能与 active
Worktree 路径或任何保留的 clone 路径重叠；clone 失败不释放预留。

进度保留 Reserved、DirectoryCreated 或 Dispatched 事实。创建后的目录身份固定，Unknown
保留原阶段，不能倒退为 Reserved。SQLite 将原生目录身份视为不透明证据，须由文件系统所有者
采集、核实，不能凭数据库行推断归属。`ProcessJournal::manage_clone` 只在目录创建证据落盘后
接管；每个 clone 执行最多登记一次变更 Run。失败后重试必须使用新身份及新目录。

终态与原始事件在同一事务提交，结果输入／资源／路径必须匹配原记录。已派发尝试须有且仅有
一个已观测退出码，且不存在未清理 Run；成功要求零退出码；校验也可将零退出码的 tag checkout 判为缺少分支。缺少退出／清理证据
时保持可恢复，不提交终态。真实仓库事实检查是运行时的额外义务，SQLite 不提供该证明。
派发前失败可按自有残留事实提交，不编造 Run。

查询不确认事件。`pending_events` 合并两种业务；确认须精确匹配原 operation／execution、Node
及 sequence 1，只删除投递记录，结果、输入及目录预留继续保留。终态或 outbox 写入失败一起回滚。

schema v3 事务迁移可识别的 v1／v2 表，保留 Worktree 旧记录／事件和未清理进程尝试，拒绝未知
结构。已有旧程序拒绝新版本。`tests/repository.rs` 与 `tests/repository_migration.rs` 使用真实
SQLite 验证这些存储义务，不证明远端 Git 执行、目录归属、Controller 接管或真实旧程序运行。
