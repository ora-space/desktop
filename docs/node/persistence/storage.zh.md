# Node 本地存储

[English](storage.md) | 中文

调用方显式注入绝对路径 `NodeConfig.home_directory`，没有从 HOME 推导的默认目录。
数据库固定为 `home_directory/ora-node.sqlite3`。托管部署要求目录仅所属用户可访问，且与 host 状态目录
不同；不会静默修改已有目录的权限。

与 process 子系统组合时保留此文件名。Node 拥有业务执行、资源与事件记录，host 和
guardian 分别拥有独立日志。OS 锁排除另一个 Node 数据库所有者，但不证明旧 Git 进程已停止。
SQLite 使用 workspace 统一的 bundled 版本，不导入旧引擎或 host schema。

数据库打开期间持有独占 OS 文件锁。SQLite 使用默认 rollback journal 和 FULL 同步写入。
新库的 application ID 为 `0x4f52414e`，schema version 为 3。精确 v1／v2 结构在身份与完整性校验后事务迁移，
保留执行、结果与待确认事件。已有空文件、其他数据库、不支持的版本、
目录和损坏数据库均拒绝打开，不自动重建。重开保留 NodeId，每个 Node 运行实例生成新的
NodeIncarnationId；显式配置身份不匹配时初始化失败。

`ora-node-db` 管理 `node_metadata`、`executions`、`resources`、`outbox`、`managed_executions` 和 `process_attempts`。
v3 新增 `execution_identities`、`clone_executions`、`clone_outbox` 和 `process_outcomes`。
共享身份表阻止 clone／Worktree 复用身份；进程关联改为引用该表，迁移保留原关联和 RunSpec，
不编造退出结果。旧 v2 程序会在迁移前拒绝 v3，降级不重置或重建此文件。
clone 持久边界见[仓库获取](repository-acquisition.zh.md)。
完整命令与解析后的目标分别存储；目标冻结规范路径绑定、授权根、任务路径、分支和 base commit。
operation／execution 唯一约束阻止身份改绑。Git 开始前预留 active 资源的 Workspace、路径和仓库内分支。
路径预留同时拒绝相互包含的重叠路径。删除引用既有归属，完成后保留 tombstone。

变更派发前，进程关联日志以有界 MessagePack 保存 execution、原 host 目录／UID、Scope／Run 身份和
精确 RunSpec。窄接口的第二条 SQLite 连接共享原 OS 锁，不形成另一个 Node 权威；任一关联句柄尚在时
仍保留该锁。确认收尾不删除历史关联。关联落库失败不得启动；收尾确认落库失败须重新查询原 Scope。
`CleanupCreation` 阶段持久记录仅剩任务分支时的清理意图。

迁移不为旧版直接 Git 执行编造进程证据。没有托管关联的旧执行若已开始，保持受阻；只有尚未进入变更阶段
的 Accepted 执行可以开始托管。数据库锁本身永远不能授权清理。RunSpec 环境值属于私有数据，不是日志。

带前置检查的转换保存 Accepted、Running、Unknown 和 Completed 证据。完成事务一起提交资源事实、
终态结果和原始事件。状态读取不确认事件。确认必须精确匹配 Node、operation、execution 和 sequence 1，
只删除投递记录；结果和执行去重在确认后仍保留。

`WriteGuard` 提供真实 SQLite 事务的故障注入点。`ora-node-db` 单元测试覆盖独占归属、文件保护、去重、
资源预留、事务回滚、重开和确认。测试入口：`cargo test -p ora-node-db -p ora-node`。

启动时还会校验表、索引定义及外键完整性，schema 标识本身不能授权未知结构。已确定无副作用的 Worktree 创建失败
退役预留并释放 active 唯一约束，保留执行去重和失败结果。未确定的执行继续持有预留。
