# Effect

[English](effects.md) | 中文

**Effect** 是 Ora 必须在某个 Workspace 中为使用它的 Agent 达成的事实。安装 Skill 是内置的 Effect 种类：已安装的 Skill 被物化到每个符合条件的 Workspace，让 Agent 能发现它们。通用机制在 `ora-effect`，Skill 集成在 `ora-effect-skill`，持久化在 `ora-db`。相关决策是 [Effect 系统基础](https://github.com/ora-space/specs/blob/main/decisions/desktop/core/effect/0-effect-system-foundation.md) 和 [Effect 审计时间与业务时间](https://github.com/ora-space/specs/blob/main/decisions/desktop/core/effect/20260905-separate-audit-and-business-time.md) 两份 ADR。

MCP 刻意不是 Effect：它是没有 Workspace 文件的会话运行时输入，见 [Session MCP](session-mcp.zh.md)。

## 模型

- 一个 Workspace 是一个 `EffectScope`，拥有一份完整的 `DesiredState` 代。Desired State 是以 `DesiredEffectIdentity` 为键的规范化集合；替换使用代的 compare-and-swap，完全相同的规范化 no-op 不推进代。
- `EffectSource` 在多次发布之间保持稳定；每次发布产生一个不可变的 `EffectRevision`。退役一个 source 不会让历史或未完成的操作失效。
- 每个消费 Effect 的运行时（Agent 插件）声明一个 `Consumer` revision。Consumer 与 Scope 配对产生一个独立的 `EffectTarget`，它是调度和就绪的单位。
- Target 绑定到 `EffectResource`，后者是独立的观测、加锁和变更单位（对 Skill 来说是 Workspace 中的一个目录）。Target 与 Resource 是多对多的：多个 Target 可以共同贡献一个物理 Resource。
- `TargetProjection` 是一个 Target 在某个 Consumer revision 下完整、确定的视图；`ResourceProjection` 合并绑定到该 Resource 的所有活跃或退役中的 Target。物化契约从 Consumer 的 Resource 模板原样传到绑定和需求上；声明了不同契约的贡献方会阻塞规划，而不是隐式选择其中一个。
- `ManagedItem` 是持久的变更授权。`ObservedItem` 只是适配器的声明，没有精确的账本和标记证据的条目即使字节与 Desired 一致，也仍是 `PreservedItem`。
- Target 的水位满足 `ready <= applied <= observed <= desired`。Resource 没有就绪水位，因为只有 Consumer 才能证明自己能消费一个完整的投影。

每个 Workspace 有一个 Scope。项目和任务的 Workspace 仓储在 Workspace 事务中创建 Scope 并播种其 Desired State；发布新的 Skill source 会播种已有的 Scope。

## 收敛

每个 Target 拥有一个合并过的、电平触发的 `ReconcileRequest`。worker 的 claim 为 Target 状态变更设置栅栏，每个绑定的 Resource 还要再获取自己单调递增的 claim，观测结果才能支撑状态或就绪。唤醒原因只用于诊断：获得授权之后，worker 总是重新加载当前的 Desired、声明、状态和账本。

1. 重新加载 Target 声明，并按稳定的身份顺序 claim 每个绑定的 Resource。
2. 在这些 claim 之下重新加载所有可变事实，投影 Target 和每个共享贡献方。
3. 在栅栏之下观测并规划每个 Resource，从不因观测而授予所有权。
4. 在任何外部副作用之前，持久化不可变的 Attempt、投影、准备好的 Operation 和 Artifact 授权。
5. 在外部调用之间持久化 Consumer 协调回执，以及 Attempt 和 Operation 的每个单调阶段。
6. 在适配器精确验证之后，原子地完成操作日志、所有权账本、状态、就绪、Condition 和请求。

Scope 的代就是收敛纪元：投影以 `(target_id, generation, consumer_revision_id)` 标识，所以任何改变某一代 Target 投影内容的事都会开启新纪元。替换 Desired State 会推进它，拓扑变化也会：退役一个 Consumer（卸载插件）或替换 Target 声明（更新插件）会在同一个事务里推进所有受影响的 Scope，记录一条 `topology_changed` 审计事件并唤醒所有 Target。观测、重试、Condition 和运行时状态变化从不推进纪元。

- 未变的 Consumer 声明什么都不触碰。新 Consumer 立即与已有的 Workspace 配对，每一轮 worker 还会把已有的 Consumer 与之后创建的 Workspace 配对。
- 变化的声明会退役旧 Target，并在新纪元创建替代者，所以更新与卸载以同样方式收敛。退役中的 Target 保留绑定，向共享合并贡献一个空的 Desired 集合，只有在这个空投影收敛之后才删除。
- Workspace 目录已不存在的 Resource 观测为空，所以在没有其他贡献方需要那里的内容时，退役可以完成。变更路径从不重建用户删除的 Workspace 根目录：如果仍有存活的 Target 指向它，两个 Target 都会持续失败，直到目录回来或那份意图被撤回。
- 重新发布一个曾经退役的 Skill source 会在每个活跃 Scope 中恢复它的 Desired 意图，所以重装一个插件版本会重新收敛。

较新的唤醒会保留活跃的 claim，让较早的那一轮完成它精确的投影。提交时，Target 状态把最新的 Desired 水位与已证明的进展合并：较早的完成不能降低任何水位，也不能在还有较新工作时报告 `Current`，较新的请求保持待处理，留给下一轮。

## 文件系统安全与恢复

文件系统 Resource 是 Workspace 根目录加上经过校验的可移植相对路径；适配器拒绝逃出 Workspace 的链接或祖先目录。每个受管目录包含 `.ora-managed.json`。变更要求数据库账本、标记身份、原生身份和上次应用的指纹完全一致。缺失的受管目录可以重建；无主、标记不匹配或已漂移的目录从不被默默覆盖或删除。

每次变更都有持久的 `Prepared → Applied → Finalized` 日志，带有精确的预期状态和计划状态。日志存在之前的瞬时失败进入计数的重试计划。日志一旦存在，被中断或结果不明的操作会在 worker 租约过期后，让它的 Attempt、Target 和 Resource 进入 `RecoveryRequired`，并带上阻塞性的人工 Condition；仍然有效的 worker 从不被隔离。调度器排除正在恢复的 Target，而不是猜测或规划第二个操作；暂存和备份 Artifact 一直保留到获得精确的清理授权。没有任何机制会自动清除 `RecoveryRequired`；它唯一的出口是受控的 `recoverEffectTarget` 命令：在关闭之前，它用适配器逐个证明被隔离的操作。外部状态恰好等于其精确计划状态的操作被终结并保留其所有权账本行；仍等于其精确预期基线的操作以 `FinalizedWithoutEffect` 关闭且不授予任何所有权，因为它的变更可证明从未落地。两个状态都证明不了的操作让 Target 保持隔离——恢复从不重放变更、从不授予未经证明的所有权、也从不改写不可变的投影历史。关闭最后一个日志会清除该 Target 的人工 Condition、让其请求回到待处理，由普通收敛对着当前 Desired State 继续推进；退役中的 Target 还会推进其 Scope 纪元，让空的退役投影落在新的不可变身份上。恢复一个 Target 从不清除共享 Resource 上另一个 Target 的隔离；该 Resource 的证据仍指向一个仍被隔离的操作。

恢复在不创建目录的前提下重新校验 journal 固化的 Workspace 与 Resource 路径，解析后的根目录必须与 journal 一致。普通文件、链接、不可访问路径和被重定向的根目录不能证明 Missing；安全的 Resource 缺失可以。恢复调用和刷新归 effects 数据 owner，只刷新曾观察到该 Target 的查询，不改变其他 Workspace 和 Agent 的 readiness 缓存。

## 审计时间与业务时间

Effect 仓储拥有行的审计时间：它们在获取写事务之后从注入的 `TimestampSource` 取样，并在整个事务中复用这个样本。每一行保留已有的 `updated_at` 以及 CHECK 约束要求的业务下界，所以墙钟校正可以让审计时间相等，但从不让它倒退，也不授予代或栅栏授权。

reconciler 在准备时、应用后和就绪后取样业务时间；同一个 Attempt 中的操作共享准备时间。恢复只在 Operation 为 `RecoveryRequired` 时记录独立的 `detected_at`。关闭事务先把每个 Operation 的原检测时间、身份和关闭结果复制到 `recovery_finalized` 审计事件，再清空其当前阶段的 `detected_at`；事件的 `occurred_at` 是关闭事务的审计时间，不是检测时间。审计写入失败时关闭也一起回滚。worker claim、恢复扫描和重试调度都取新样本，重试延迟从处理失败时开始计算。回执的 `received_at` 表示本地持久接收。Condition 保留其身份和首次观测时间，最后观测时间从不倒退。状态读取在一个快照中同时返回审计元数据和领域证据。本地 Skill 的 `updated_at` 仍是它的 revision key；未变化的发布不产生 revision。

## 接口

- `getEffectTargetStatus` 向前端暴露 Target 的 desired、observed、applied 和 ready 代，恢复身份和当前的 Condition。
- `retryEffectTarget` 只唤醒一个 Target，不改变 Desired State。发送门控把 Resource 级 blocking Condition
  （例如旧技能目录造成的 preserved item 冲突）显示为可见失败并列出冲突名称；用户移除外部原因后，该命令
  重新唤醒 Target，并且只刷新它的 readiness 缓存。
- `recoverEffectTarget` 是 `RecoveryRequired` 的受控出口；当发送门控被恢复隔离阻塞时，对话界面提供这个入口，拒绝恢复时隔离保持不变。
- 插件注册暴露 `effectResources`。Agent Consumer 实现 `effect/coordinate`、`effect/reactivate` 和 `effect/verify_ready`；这些载荷留在 `ConsumerAdapter` 边界之后，不给通用机制增加 Agent 专有的阶段。
- 包适配器在发布之前计算不可变的指纹；SQLite 保存摘要和指纹，但从不通过 Resource 适配器读取包目录。
- Skill Effect 变更与 MCP 刷新共用 Agent Session Barrier，见 [Session MCP](session-mcp.zh.md#实时刷新)。

Schema：迁移 `0006`（Scope、Source 与 Revision、Desired State、Consumer 与 Target、Resource 与绑定、投影、所有权、状态、Condition、请求与 claim、Attempt、Operation 与 Artifact、回执、审计事件）、`0010`（独立的恢复检测时间）和 `0019`（`finalized_without_effect` 关闭阶段，通过暂存副本重建操作日志引入）。见[持久化](../core/persistence.zh.md)。

## Windows 文件占用与诊断

读取程序打开文件时若没有允许删除共享，Windows 会阻止其父 Skill 目录改名，返回拒绝访问（5）或共享/锁冲突（32/33）。目录交换对这些 Windows 错误按 50 ms 间隔重试，每次改名最多等待一秒；其他错误与其他平台立即失败。持续占用和 ACL 限制仍安全失败：Ora 不终止读取程序、不修改权限、不删除目标，也不改为覆盖复制。应先停止占用 Skill 文件的活动，再尝试显式恢复。

恢复成功表示 journal 已关闭，并不表示 readiness 已达成：`finalized_operations` 与 `closed_without_effect_operations` 分别表示精确 planned 和精确 baseline 证明的关闭数量。访问限制仍在时，新的收敛仍可能失败。按以下日志串联定位：

- `effect_recovery`：Target 身份、开始和关闭计数；后续仍需普通收敛。
- `effect_reconcile`：每轮独立的 `trace_id` 将 Target、适配器事件与最终结果关联起来。
- `skill_materialization`：不可变 operation/attempt/Resource 身份、generation、mutation、stage、outcome 和耗时。`stage_package`、`verify_planned`、`verify_expected` 区分复制校验与状态证明。`backup_previous`、`install_staging` 记录改名前后路径、成功尝试次数或失败的 OS 错误；IO 错误链标明具体 action。
- 回滚和 Artifact 清理失败均记录日志，不再静默丢弃。

日志不包含 Skill 文件正文、ownership marker 原文或凭据。本地诊断日志中的文件路径可能标识用户 Workspace，对外公开前应脱敏。
