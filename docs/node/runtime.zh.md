# 独立 Node 运行入口

[English](runtime.md) | 中文

> 当前目标已调整为[clone 指定仓库与分支](minimal-loop.zh.md)。下文描述已有 Worktree 运行入口，
> 不表示独立 Node 已提供 clone 命令或新闭环。

Linux `ora-node` 可执行程序持有显式配置的 Node 数据库，恢复待处理的 Worktree 执行并处理正常停止。
它**不安装 Controller IPC**，不从 stdin 或文件接收新命令，也不切换既有 Backend 写入入口。
进程内调用方使用 `Node::open(config, process_config, shutdown)` 及原有类型化 Node 方法。

## 部署

构建 `cargo build -p ora-node -p ora-process-host -p ora-process-guardian`，先部署并启动
[独立 process host](../process/host/service.zh.md)，再运行：

```text
ora-node /absolute/path/node-config.json
```

未注册仓库的空闲 Node 可使用以下配置：

```json
{
  "node": {
    "home_directory": "/home/alice/.ora/node",
    "identity": "Discover",
    "repositories": []
  },
  "process": {
    "host_directory": "/home/alice/.ora/process",
    "expected_uid": 1000,
    "git_program": "/usr/bin/git",
    "environment": { "PATH": "/usr/bin:/bin", "HOME": "/home/alice" },
    "command_timeout_ms": 30000,
    "cleanup_timeout_ms": 5000,
    "shutdown_grace_ms": 2000
  },
  "timezone": "Asia/Shanghai",
  "recovery_interval_ms": 1000
}
```

路径、UID 和时区按实际部署替换。`Discover` 保留已有 NodeId，首次初始化时生成身份；
`{"Require":"registered-node-id"}` 要求与已注册身份一致。仓库绑定使用 `RepositoryBinding`，包含
仓库引用、已有 Main Workspace 身份／路径、授权根和 worktree 根；不克隆仓库，也不从 cwd 猜测绑定。

Node 数据固定放在 `home_directory/ora-node.sqlite3`。Node 与 host 的状态目录均为显式注入的绝对路径，
不从 HOME 推导；上面环境中的 HOME 只影响 Git。Unix 下新 Node 目录为私有目录；已有非私有目录、
受信路径中的符号链接及 host 状态目录的别名会被拒绝，不会被 chmod、覆盖或挪作他用。
已有 v1 Node 数据库按[存储迁移规则](persistence/storage.zh.md)处理。

## 执行与恢复

所有 Git 调用均通过 host／guardian 使用显式的程序和环境。只读前置检查没有业务变更关联，复用有界
只读 Scope。每次变更之前，Node 先在自己的数据库提交 execution／host／Scope／Run 关联；host 和
guardian 各自保留意图与事实。回复丢失不授权新建另一次尝试。

恢复先关闭每个未收尾的原变更 Scope，观察到关闭完成后才检查或修复资源。guardian 不可用或收尾
无法确认时，原执行保持 Unknown，资源预留继续生效；进程内接口仍允许提交不冲突的其他工作。
迁移而来的旧版直接 Git 执行若已开始、又没有进程关联，继续受阻；升级或取得数据库锁都不能证明旧进程已停止。

仅剩所属分支且仍位于冻结基准的创建执行进入持久 `CleanupCreation`，只清理所属残留，每轮恢复至多
按原输入和 base commit 重试一次。其他 checkout 占用、分支内容改变及无法解释的非空残留不会被强制修复。
完整且归属匹配的工作树即使有新提交，也完成创建，不重置提交。

guardian 在 exec 前固定请求方 Node 的进程观测身份。Node 退出会触发 Run 收尾，即使 host 已断连。
这仍是无 root 的 **BestEffort**，不是强纳管：逃逸后代或 guardian 死亡可能导致无法确认收尾。
所属进程的存活状态只触发清理，不授权资源恢复；没有引入认证令牌或租约。

SIGTERM／SIGINT 关闭新工作准入，给当前 Git 配置的完成宽限，再请求收尾。停止会核实未完成变更 Scope
与只读 Scope 的关闭结果，无法确认时报告失败，不伪报清理完成。业务对账可以留到下次启动。
输出有界且易失；截断、失败或丢失的输出不能作为完整 Git 事实解析。

## 验证与下一阶段

`cargo test -p ora-node --test standalone` 使用真实 Node、host、guardian 程序、SQLite 和阻塞 Git hook。
定向测试前先构建三个可执行程序。覆盖 Node SIGKILL 后立即重启、没有新 Node 时 guardian 独立收尾、
guardian 丢失后不冲突工作继续执行、正常停止超时及数据目录隔离。单元测试覆盖局部恢复故障、冻结基准、
结果落库、归属和重放；`cargo test -p ora-node-db` 覆盖迁移与进程关联写入故障。

Controller 投递／结果接管、Backend 切换、非 Linux 托管 adapter、Strong、持久输出、stdin 及进程历史／
guardian 退休仍不在本切片内。现有进程历史保留，不表示完整 process ADR 蓝图已经完成。
