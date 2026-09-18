# Node Worktree 执行与持久化实施计划

[English](worktree-execution.md) | 中文

> 2026-09-18：当前最小闭环已改为[clone 指定仓库与分支](../minimal-loop.zh.md)。
> 本文保留 Worktree 已实现能力与原计划，不再代表首条闭环的后续实施顺序。

## 当前集成边界

`node-db` 工作已导入 `node-process`，保留当前 workspace 的 SQLite 依赖和既有
`home_directory/ora-node.sqlite3` 布局；Node 业务记录不并入 `host.sqlite` 或 `guardian.sqlite`。

R2 已修正：有效且归属匹配的 checkout 即使有新提交，也按创建成功处理，原 `base_commit`
仍是比较基准。R5 已修正：新清理请求引用已退役归属、且资源已重新出现时，保存明确的
WorktreeConflict 失败，不碰现有资源。R3 已补测试：未确认的创建／删除事件重开后同时保留，分别确认互不影响。

R1 已允许不冲突的新工作继续执行，未知执行仍保留资源预留。仅剩所属任务分支且仍位于冻结基准时，
按原执行持久记录清理意图，再清理重建；每轮恢复至多重试一次。分支内容改变、被其他 checkout 占用，
或存在无法解释的非空残留时仍保护现场。
R4 已通过 [process host](../../process/host/service.zh.md) 执行 Git。Node 在派发变更前持久保存
Scope／Run 关联，恢复时关闭原尝试并确认收尾后才对账资源；guardian 在 Node 退出时独立终止所属 Run。
guardian 证据不可用时继续阻塞冲突工作。BestEffort 不是 Strong 静止保证。
[独立 Node](../runtime.zh.md) 已提供启动恢复和有界停止，不含 Controller IPC。

Node IPC、Controller 结果接管属于后续步骤。当前信任本机调用方；认证、秘密令牌、租约及
特权 Strong 已推迟，不作为闭环前置。现有 Backend 写入入口不变，本次不改变 ADR 状态。

本 PR 已在 `apps/ora-node` 实现 Worktree 执行与恢复，在 `crates/node-db` 实现 Node 本地持久化，
通过进程内接口完成创建、删除、查询、结果重放与确认的闭环。本文保留实施顺序和检视基线；
已完成事项打勾，最终接口、测试证据及实现取舍见第 7 节。

本 PR 对应本机 Worktree 最小闭环的第 2 步：`feat(node): execute worktree operations with durable recovery`。
前一步已提供 `ora-node-protocol` 消息和 Frame Codec；本步已提供独立 Node 生命周期，下一步接入本机 IPC 和 Controller。
本 PR 内部按下文顺序实施、检视，Git 副作用与持久去重必须一起交付。

## 1. 目标、依据与范围

完成后，调用方应能向一个绑定了已有 main worktree 的本机 Node 提交 `EnsureWorktree` 或
`RemoveWorktree`，查询执行状态，并在响应丢失或 Node 重启后继续使用原执行身份获得结果。
Node 必须先持久保存输入再变更 Git；无法证明副作用结果时，保留恢复状态。

实施依据：

- [Node Worktree 根决策](../../../specs/decisions/node/worktree/0-worktree-management.md)：执行归属、已有 main worktree 前置条件及任务资源的清理范围。
- [Controller–Node Protocol 根决策](../../../specs/decisions/node/protocol/0-controller-node-protocol.md)：操作／执行身份、结果保留、查询、重放与确认。
- [当前协议实现](../../protocols/controller-node-protocol.zh.md)：本 PR 使用的消息、状态和错误码。
- [现有任务 Worktree 行为](../../task-worktrees.md)：已证明归属的任务资源使用 force 清理，以及残留目录的处理语义。

相关 ADR 按最小闭环约定保持 `proposed`，随实现调整，在完整闭环合入 main 后统一转为 `implemented`。
实现中若改变本文的范围、事务顺序或恢复语义，应同步修改中英文 plan 并说明原因，供 reviewer 检视。

| 本 PR 交付                                             | 后续工作                                                            |
| ------------------------------------------------------ | ------------------------------------------------------------------- |
| `ora-node` library、独立可执行入口、初始化、恢复和停止 | Controller IPC、握手及重连调度；认证延期                            |
| `ora-node-db` 独立 SQLite 数据库、schema 与事务接口    | Controller 业务数据库、任务调度与 Client 状态展示                   |
| 注入并校验本地仓库、Main Workspace 绑定和授权根        | Node 注册、仓库发现、绑定管理界面                                   |
| 创建／删除、状态查询、待确认事件读取及确认处理         | Transport 实际投递、Controller 持久接管后发送确认                   |
| 本地持久恢复、去重与聚焦验证                           | clone、fetch、push、跨 Node 迁移、文件传输及旧 Backend 写入入口切换 |

本 PR 不承诺外部 Git 命令严格恰好执行一次。进程可能在 Git 已变更、结果尚未落盘时停止，恢复必须对账；
不能仅凭执行记录、命令退出码或路径存在就认定成功。

## 2. 代码落点与接口

| 落点                                 | 本次实现职责                                                            | 接口应封装的内容                                                         |
| ------------------------------------ | ----------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `apps/ora-node`，包名 `ora-node`     | 初始化、请求校验、本地资源解析、Worktree 执行、恢复及协议结果映射       | 调用方提交命令、查询、读取待确认事件、确认事件；无需编排 SQL 或 Git 步骤 |
| `crates/node-db`，包名 `ora-node-db` | Node 身份、执行记录、资源归属、结果和待确认事件的 SQLite 存储           | 条件接受执行、状态转换、原子完成、查询、恢复扫描和精确确认               |
| `crates/gitlancer`                   | 复用类型化仓库检查、worktree 创建／删除、分支删除；按需补齐事实检查能力 | 封装 Git 命令与解析，不包含 Ora 操作身份、数据库或恢复策略               |
| `crates/utils`                       | 复用路径校验、规范化和包含关系能力；通用缺口补在这里                    | 不引入 Ora 领域词汇或其他 `ora-*` 依赖                                   |
| `crates/node-protocol`               | 复用当前消息和结果类型                                                  | 必要时开放可复用的语义校验入口；不扩展新执行能力或绑定 SQLite            |

依赖方向为 `ora-node -> ora-node-db / ora-node-protocol / gitlancer / ora-utils`。
`ora-node-db` 可以复用协议身份与结果类型，但不依赖 `ora-node`、`ora-db`、Controller、Git 或 Transport。
不把 Node 执行表加入现有 `crates/db`，不复制其业务 repository 和迁移历史。

`ora-node` 先提供 library 入口，供本 PR 测试及下一步进程入口共同调用。内部按初始化、执行、资源解析、
恢复组织私有模块；`ora-node-db` 封装 schema、迁移和事务实现。具体文件名、表名与函数签名可在实现时确定，
但调用方不能通过通用 CRUD 绕过状态转换或自行拼装完成事务。

Git、存储故障和时间通过可注入接口控制，优先使用 trait 与泛型。Git 使用真实和 fake 实现；
数据库测试以临时 SQLite 文件重开验证持久性，内存数据库或 fake 仅用于补充确定性的失败注入。

## 3. 实施顺序

### P1：建立两个包与初始化入口

- [x] 将 `apps/ora-node` 和 `crates/node-db` 加入 Cargo workspace、默认成员及必要依赖声明。
- [x] 定义进程内初始化输入：`home_directory`、本地仓库／Main Workspace 绑定、授权根和 Node 管理的 worktree 根。
      调用方显式提供绝对数据目录 `home_directory`，不从 HOME 推导默认值；数据库固定放在其下的 `ora-node.sqlite3`。
      测试注入临时目录，不依赖进程环境；进程启动参数留给 IPC 步骤。
- [x] 持久保存 `NodeId`，每次运行生成新的 `NodeIncarnationId`。重开同一数据库沿用 NodeId；
      请求或配置中的 NodeId 不匹配时拒绝执行，不改写旧记录的归属。
- [x] 定义单个 Node 数据库的执行所有者，阻止两个运行实例同时驱动其中的执行。
      首版允许串行执行 Git 变更；无需实现通用多仓库并行调度，但同一仓库的创建、删除与恢复不能交错执行。
- [x] 明确初始化失败、恢复待处理和可接收新工作的状态；恢复入口可重复调用，失败时不绕过恢复直接执行。

完成条件：测试可在临时目录初始化、关闭并重开同一 Node；身份与配置冲突可被观察；不需要启动 IPC。

### P2：实现独立 SQLite 存储与事务

- [x] 定义 Node 专属 schema 标识、版本及迁移入口；只升级可识别的 Node 数据库。
      若配置位置已有目录、其他数据库、损坏或不支持的 schema，返回明确错误并保留原内容，不覆盖或自动重建。
      新文件布局不能占用旧版 Ora 已使用的文件／目录路径。
- [x] 将下表数据持久化，并通过类型和数据库约束共同保护身份、归属及状态完整性。
- [x] 实现原子“接受或读取既有执行”，避免先查询后插入造成重复接受；首个闭环为一个 `operation_id`
      绑定一个 `execution_id`，不能换 execution 绕过去重。
      在接受事务中记录资源预留／归属及执行关联，防止不同身份并发占用同一 worktree、路径或分支；不能等 Git 成功后才记录归属。
      创建只预留未占用目标；删除引用既有归属记录，不能通过接受删除请求取得现有资源的所有权。
- [x] 实现带前置状态检查的更新；原子提交资源事实、终态结果和待确认事件。
      Git 命令不在 SQLite 事务中运行，持久化提交成功后才允许进入下一次外部变更或输出结果。
- [x] 实现按原身份查询、扫描待恢复执行、读取待确认事件和精确确认；确认清理投递记录后仍保留执行去重记录和结果。

| 持久数据       | 必须保存的信息                                                                               |
| -------------- | -------------------------------------------------------------------------------------------- |
| Node 元数据    | 持久 NodeId、schema 身份与版本                                                               |
| 执行输入       | operation／execution 身份、操作类型、原关联 request_id（若有）、完整规范化请求               |
| 已解析执行目标 | 仓库与 Main Workspace 绑定快照、授权根及目标路径、实际分支、创建前解析出的不可变 base commit |
| 执行进度       | 当前状态、执行／观察实例、已知阶段及最近恢复诊断，足以区分未开始和可能发生过变更             |
| 资源归属       | Node／任务 Workspace／worktree 身份与仓库、路径、Ora 创建分支的关联；删除后保留去重所需证据  |
| 结果与事件     | 原始结构化结果、产生该结果的运行实例、执行内 sequence、待确认状态及重放所需 metadata         |

规范化请求用于参数比较，已解析目标用于实际执行与恢复。必须分别保存：重传不能重新解析已移动的
`base_ref` 并覆盖原 base commit，也不能因启动配置改变而悄悄改到另一条路径。
不透明身份保持原值，不通过 trim、大小写转换或路径解析合并不同身份。

完成条件：真实 SQLite 测试证明唯一约束、事务回滚、重开恢复和确认后的结果保留；具体 schema 与文件布局有可检视说明。

### P3：实现本地资源解析与所有权校验

- [x] 将 `RepositoryRef` 解析到初始化时提供的本地注册项；请求中的 Main Workspace 身份和 Node-scoped path
      必须匹配该注册项，并用 Git 证明它是相应仓库的有效 main worktree。
- [x] 复用 `ora-utils::path` 校验 `NodeManaged.directory_name`，本 PR 将它作为授权 worktree 根下的一个目录名；
      拒绝父目录穿越、绝对路径、平台保留形式及多级子路径。通过 `Path`／`PathBuf` 构造目标，校验已有前缀及静态符号链接逃逸。
- [x] 校验 main worktree 和目标路径的授权关系，拒绝目标与 main checkout 重叠、占用授权根本身、
      或与另一资源的路径／分支冲突。路径通过校验不等于资源属于 Ora。
- [x] 创建前解析 base ref 到 commit 并保存；恢复时使用保存值。删除使用记录中的资源归属，
      不要求任务分支仍停留在创建时的 commit，也不因原 base ref 后来移动或消失而删除其他资源。
- [x] 在每次变更及恢复前重新检查实际仓库、worktree 注册和资源归属；非空且无归属证据的目录保持原状。

完成条件：非法路径、错误 Node／仓库／Main Workspace、同名分支及资源冲突均在 Git 变更前被拒绝。
共享路径工具不保证防御校验与使用之间的恶意符号链接替换；本 PR 不把静态包含校验宣称为操作系统级隔离。

### P4：接通创建、删除与持久结果

- [x] 进程内入口执行消息语义校验，不能依赖 Frame Codec 已检查输入。先检查既有执行身份及输入，
      已完成重试直接读取原结果，不依赖当前仓库、base ref 或路径仍可访问。
- [x] 新请求按“校验并解析 → 持久接受 → 持久标记可能开始变更 → Git 执行 → 事实检查 → 原子完成”的顺序处理。
      写入失败时停止；副作用之后写入失败时保留可恢复记录，不提前返回成功。
      对身份合法但执行前置条件不满足的请求，原子保存输入、结构化失败及事件后返回；此类记录不要求有已解析目标。
      消息格式错误和身份冲突作为入口拒绝处理，不覆盖已有执行或伪造其终态事件。
- [x] `EnsureWorktree` 使用持久保存的路径、分支和 base commit 创建 linked worktree。
      成功结果的路径、分支来自 Git 观察，base commit 来自创建前已验证并持久保存的 Git 解析结果。
      核对 `gitlancer::create_worktree` 的失败补偿路径，确保其中的 worktree／分支清理也受同一执行身份和所有权约束。
- [x] `RemoveWorktree` 组合现有 `gitlancer::Git::delete_worktree` 和 `delete_branch`，依次移除 linked worktree
      与 Ora 为该任务创建的本地分支；每个阶段都可检查和恢复。沿用现有任务清理的 force 语义，显式使用类型化删除模式，
      仅清理已证明归属的 linked checkout 和本地任务分支，不以 force 绕过 main worktree、其他 worktree 占用或归属检查。
- [x] 删除完成前验证 worktree 注册、目标目录及该任务所属的本地分支均已不存在。
      有 Node 归属记录的空目录可清理；非空残留、分支被其他 worktree 使用、或归属冲突时，报告结构化失败或保留未知状态。
      main worktree 删除必须显式拒绝。
- [x] 原子保存终态及原事件；通过进程内接口读取待确认事件，按精确身份确认，重放保持原 sequence 和结果。

`AlreadyAbsent` 仅用于已经证实完整清理目标都不存在的情况。仅 linked checkout 不存在、任务分支仍存在时，
必须继续有归属证据的分支清理；无法完成时不得产生 `WorktreeRemoved`。

完成条件：两个命令经同一进程内入口完成持久闭环；重复请求不会重复启动变更；部分删除不会被记录为成功。

### P5：实现状态查询、重放与崩溃恢复

- [x] 按第 4 节状态规则扫描并对账所有未完成记录，覆盖 Git 未开始、部分完成、完成未落盘和结果已落盘的窗口。
- [x] 恢复保留 operation／execution 身份和已解析目标；不能创建新的执行或重新挑选路径、分支、base commit。
- [x] 恢复检查完成前阻止相关新变更；未知记录继续阻止其资源上的冲突操作。
      查询、待确认事件读取和再次恢复应仍可用，不因一条未知记录丢失其他已完成结果。
- [x] 状态查询只返回执行证据，不确认事件、不清理投递记录，也不启动 Git。
      待确认事件由独立接口枚举；下一步 Transport 据此主动重放，无需先查状态。
- [x] 校验确认中的 Node、operation、execution 和 sequence；重复精确确认幂等，错误或未来序号不能清理其他事件。
      已确认执行仍可查询原结果，不重新生成待确认事件。

完成条件：关闭并重开数据库后，原输入、结果与未确认事件可恢复；事实不充分时返回 `Unknown` 并保持可再次对账。

### P6：补齐检视证据与交接

- [x] 为第 5 节每项验收条件提供直接测试证据，记录实际测试入口；未完成项保留未勾选状态。
- [x] 同步中英文 plan，补充最终接口、schema／布局和必要的实现取舍；若语义改变，同步检查相关 ADR。
- [x] 运行相关 crate 测试和 lint，再运行完整 `task test`；将结果与剩余限制写入 PR。
      本 PR 的执行与持久化恢复必须完整，不把这些验证延期到最后的旧代码清理步骤。

## 4. 状态与恢复检视基线

内部状态按以下语义实现；名称可以调整，但不能用多个互不约束的 `Option` 表达终态：

| 内部状态          | 已知事实                         | 协议查询            | 允许的后续动作                 |
| ----------------- | -------------------------------- | ------------------- | ------------------------------ |
| Accepted          | 身份和输入已落盘，Git 尚未开始   | `Accepted`          | 持久标记 Running 后执行        |
| Running           | 已记录变更意图，Git 可能已执行   | `Running`           | 活跃执行继续；重启后先对账     |
| ResultUnknown     | 证据不足，记录及归属仍保留       | `Unknown`           | 重新观察；证据足够后恢复或完成 |
| Completed(result) | 成功或已确定失败与事件已原子保存 | `Completed(result)` | 返回原结果，按需重放待确认事件 |

无执行记录的查询也可返回 `Unknown`，但不能因此创建记录或启动 Git。已有 ResultUnknown 必须保留输入和诊断，
不能等同于“记录不存在”。本 PR 不将尚待对账的执行存为 `Completed(Failed(ResultUnknown))`，以免把不确定状态封死为终态。

新产生的恢复结果携带实际完成观察的当前运行实例。已经持久保存的结果与事件保留原实例；
`ExecutionStatus.node` 使用当前报告者实例，内层结果的持久 NodeId 必须与它一致。

| 恢复观察                                                             | 本 PR 必须实现的行为                                  |
| -------------------------------------------------------------------- | ----------------------------------------------------- |
| 确认 Git 未开始，或确认无残留变更且可安全重试                        | 使用原身份和已解析目标继续执行                        |
| 有效 checkout 的仓库、注册、分支、路径和持久归属匹配，包括已有新提交 | 完成原执行，保留原始 base_commit                      |
| 删除中 worktree 已移除，但该任务所属的本地分支仍存在                 | 重新验证归属后继续分支清理，不提前报告成功            |
| 删除全部目标已不存在                                                 | 保存 `AlreadyAbsent` 幂等成功                         |
| 能确定操作失败且没有未解释的副作用                                   | 保存相应结构化终态失败                                |
| 仅剩所属分支且位于冻结基准，没有其他 checkout 或无法解释的文件       | 持久记录清理意图，清理后按原执行重试，每轮至多一次    |
| 注册与目录不一致、配置归属改变或 Git 检查失败，不能证明结果          | 保留 ResultUnknown 和诊断，不覆盖、猜测成功或盲目重跑 |
| 终态已持久化，响应或确认丢失                                         | 返回原结果／重放原未确认事件，不再执行 Git            |

幂等检查同时约束 operation 与 execution 的关联。相同 execution 的操作类型或规范化输入改变、
同一 operation 被改绑到另一 execution，均返回 `IdentityConflict`，保留旧记录和结果；
拒绝请求不能覆盖原执行的终态或占用它的事件身份。
`request_id` 用于关联，不替代去重身份；`worktree_id` 用于资源归属，创建与删除应使用不同的 operation／execution。
同一 execution 正在运行的重复请求只返回当前状态，不再启动一次执行。

## 5. 测试与验收矩阵

| 编号 | 场景与必须证明的结果                                                                                                          | 主要证据落点                             |
| ---- | ----------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| V1   | 新库初始化、重开沿用 NodeId 且更换运行实例；其他数据库／目录／损坏 schema 不被覆盖；重复运行实例不能同时驱动执行              | `ora-node-db` 文件测试 + Node 初始化测试 |
| V2   | 首次接受或 Running 写入失败时无 Git 变更；完成事务失败不留下“有结果无事件”或提前成功；重开后仍可恢复                          | SQLite 事务测试 + Node 故障注入          |
| V3   | 相同请求顺序及并发重传只接受一个执行；改变输入、操作类型或 operation／execution 关联返回冲突；不同身份不能并发占用同一资源    | 存储唯一约束 + Node 入口测试             |
| V4   | 错误 Node／仓库／Main Workspace、非法目录名、静态 symlink 逃逸、main 路径重叠和他人资源冲突均在副作用前拒绝                   | 资源解析测试 + fake Git 调用记录         |
| V5   | 创建返回完整 Git 事实；base ref 移动、配置变化或响应丢失后的重试不替换持久输入；已完成重试无需访问 Git                        | Node 入口测试 + 真实临时仓库             |
| V6   | 在接受后、Running 后、创建变更后分别中断并重开；恢复仅在事实充分时继续或完成；仅剩分支等歧义保持 Unknown                      | 可控中断点 + 临时 SQLite 文件 + Git 观察 |
| V7   | 删除同时清理并验证 linked worktree 和 Ora 分支；仅 worktree 缺失仍清理分支；全部缺失才返回 AlreadyAbsent                      | Node 删除测试 + 真实临时仓库             |
| V8   | worktree 已删除但分支未删除时中断；恢复只继续已证明归属的剩余清理；分支删除失败不能上报 WorktreeRemoved                       | 阶段故障注入 + 重开测试                  |
| V9   | main worktree、他人分支、非空无主目录不被删除；已证明归属的任务 checkout 含未提交内容或任务分支含新提交时仍按任务清理语义删除 | 真实临时仓库与文件系统测试               |
| V10  | 结果落盘后丢弃响应再重开，重放保持原结果、sequence 和原实例；状态外层使用新实例，NodeId 一致                                  | Node + SQLite 重开测试                   |
| V11  | 查询不停止重放；精确确认及重复确认幂等；错误身份／序号不清理事件；确认后查询保留结果且不重新排入投递                          | 查询、重放与确认接口测试                 |
| V12  | 恢复与新命令不会在同一资源上交错变更；Unknown 阻止冲突工作，多次恢复不新建执行；其他结果仍可查询                              | Node 执行协调测试                        |

内存 fake 用于控制 Git 观察、存储失败和执行暂停点；真实 SQLite 重开用于证明持久性；真实临时仓库用于证明
Git 适配与文件系统结果。测试通过调用方使用的接口检验完整记录、协议结果和变更次数，不只比较 SQL 字段或命令字符串。
使用注入时钟与显式同步点，不依赖 sleep 或修改进程环境；遵守仓库的 `pretty_assertions` 和测试日志隔离约定。

实现阶段的验证顺序：

```bash
task format
cargo test -p ora-node-db -p ora-node
cargo clippy -p ora-node-db -p ora-node --all-targets -- -D warnings
```

若修改 `gitlancer`、`ora-utils` 或 `ora-node-protocol`，追加相应 crate 的测试与 lint。
最后运行 `task test`。这些命令对应本次实现；实际验证结果见第 7 节。

## 6. PR 完成条件

- [x] P1–P6 完成，V1–V12 在本 Node 切片范围内验证通过，不代表 Controller 端到端验收完成。
- [x] Reviewer 能从请求沿着持久接受、Git 变更、结果事务、重放／确认和重启恢复检查完整链路。
- [x] 数据库或 Git 任一阶段失败时，都能说明保留了什么事实、后续允许做什么；没有以新身份绕过不确定状态的路径。
- [x] main worktree 与任务资源的归属和清理范围明确，旧文件布局不会被覆盖，新旧写入入口尚未同时接管同一资源。
- [x] 下一步 IPC 可直接使用本 PR 的初始化、命令、查询、待确认事件与确认接口；无需再补持久去重、分支清理或崩溃恢复才能接入。

## 7. 最终接口、实现取舍与验证证据

### 调用入口与布局

`Node::open(NodeConfig, ProcessConfig, Shutdown)` 使用受管 Git 和 Ora 本地时钟；进程入口应先初始化 Ora logging 的时区。
测试使用 `Node::open_with_dependencies(config, git, writes, clock)`，分别注入 `WorktreeGit`、
`WriteGuard` 和 `Clock`。Node 持有 `home_directory`，不读取 `HOME`。
[存储布局与 schema](storage.zh.md)说明 `home_directory/ora-node.sqlite3`、文件保护和表约束。

- `submit(Command::Ensure(message) | Command::Remove(message))` 返回 `ExecutionStatus`。
- `status(&GetExecutionStatusMessage)` 返回当前报告实例及持久状态；不启动 Git、不确认事件。
- `pending_events()` 返回原始协议事件；`acknowledge(&EventAckMessage)` 精确确认。
- `state()` 区分 `Ready` 与 `RecoveryPending`；`recover()` 可重复对账全部未完成执行。
- `identity()`、`node_id()`、`home_directory()` 和 `repositories()` 提供只读运行信息。

所有变更入口要求 `&mut Node`；共享调用方可使用 Mutex，并发重传等待同一执行结果。
数据库文件上的独占 OS 锁阻止不同运行实例同时执行，关闭时显式解锁，避免并行 spawn 的子进程临时
继承描述符导致重开误报冲突。`RecoveryPending` 只表示存在未完成执行，不再是整个 Node 的准入门禁。
资源预留阻止冲突工作，不冲突的新任务仍可执行；原执行重传、查询、事件读取与确认均保留。

`Target` 单独持久保存 main 路径、实际 Git metadata directory、授权根、worktree 根、任务路径、
分支和不可变 base commit。运行时重复校验这些事实，配置变化不能重定向原执行。
Git 报告的 main registration 必须与绑定路径一致；不能只因某目录能运行 Git 就把它当作 main checkout。
linked checkout 还要通过 metadata directory 的 `commondir` 与 `gitdir` 回指验证。

`gitlancer::create_worktree_for_recovery` 只执行 add，由 Node 负责后续事实检查；不会在观察失败时
隐式清理现场。删除沿用 typed Force 模式，按 checkout、空残留目录、任务分支分阶段落盘与复查。
精确本地分支名不受同名 tag 影响。删除归属检查忽略可变 `base_ref`，保留其他身份与目标匹配要求。

前置条件失败原子持久保存失败结果和事件；消息格式错误、Node 或执行身份冲突只拒绝入口。
副作用后的存储错误不返回成功，剩余记录阻止新变更并允许恢复。未解释的残留和检查失败保留 Unknown，
不存为 `Completed(Failed(ResultUnknown))`。结果与原实例、原 sequence 1 一起保留，确认只删除 outbox。
已确定无副作用的创建失败会退役其资源预留，释放路径、分支和 Workspace 的 active 唯一约束；
旧执行输入及失败结果仍保留。已退役资源的 tombstone 不授权删除后来出现的同路径资源。
已确认资源重新出现时，针对退役归属的新清理操作保存终态 WorktreeConflict；真正的观察失败
仍保持 Unknown。原操作重传返回不变的历史结果。

### 直接测试证据

下面均为 crate 内单元测试；Node 的 Git 场景使用真实临时仓库，故障通过适配器及 SQLite 写入边界注入。
线程同步使用 Barrier、Mutex 与 channel，不使用 sleep 或修改进程环境。

| 验收 | 测试（省略公共模块前缀）                                                                                                                                                                                                                                                               |
| ---- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| V1   | `identity_and_exclusive_owner_survive_reopen`、`preserves_foreign_corrupt_empty_and_future_files`、`rejects_modified_schema_without_migrating_or_rebuilding`、`injected_home_persists_node_but_not_incarnation`                                                                        |
| V2   | `completion_and_outbox_rollback_together_and_ack_retains_result`、`sqlite_failures_gate_mutations_and_recover_on_reopen`                                                                                                                                                               |
| V3   | `atomic_acceptance_deduplicates_and_reserves_resources`、`duplicate_inputs_and_resource_conflicts_preserve_original_result`、`concurrent_retransmissions_mutate_once`                                                                                                                  |
| V4   | `invalid_paths_bindings_and_unowned_resources_never_mutate_git`、`existing_targets_and_main_workspace_are_protected`、`static_symlink_escape_is_rejected`（Unix）、`inconsistent_main_registration_is_rejected`, `main_checkout_overlap_is_rejected_even_for_a_distinct_task_identity` |
| V5   | `real_create_remove_replay_and_acknowledgement`、`completed_retry_ignores_moved_base_and_missing_configuration`                                                                                                                                                                        |
| V6   | `sqlite_failures_gate_mutations_and_recover_on_reopen`、`process_stops_before_and_after_create_keep_frozen_identity_and_base`、`branch_only_creation_recovers_and_preserves_other_results`、`changed_checkout_and_unavailable_git_keep_unknown_evidence`                               |
| V7   | `real_create_remove_replay_and_acknowledgement`、`missing_checkout_still_cleans_branch_and_empty_owned_directory`                                                                                                                                                                      |
| V8   | `partial_removal_continues_only_the_owned_branch`                                                                                                                                                                                                                                      |
| V9   | `existing_targets_and_main_workspace_are_protected`、`real_create_remove_replay_and_acknowledgement`、`missing_checkout_still_cleans_branch_and_empty_owned_directory`、`branch_cleanup_uses_exact_local_names_even_with_ambiguous_tags`                                               |
| V10  | `real_create_remove_replay_and_acknowledgement`                                                                                                                                                                                                                                        |
| V11  | `completion_and_outbox_rollback_together_and_ack_retains_result`、`real_create_remove_replay_and_acknowledgement`                                                                                                                                                                      |
| V12  | `recovery_and_new_submission_share_one_mutation_owner`、`branch_only_creation_recovers_and_preserves_other_results`、`configuration_change_keeps_recovery_pending_without_redirecting_mutations`                                                                                       |

测试实现位于 `crates/node-db/src/tests.rs` 和 `apps/ora-node/src/tests/`；Git 适配接口还在
`crates/gitlancer/src/git/inspection.rs` 有单元测试。

合入 `node-process` 后的验证结果：`task format`、相关 crate 测试、`--all-targets` Clippy
和完整 `task test` 在本地 Linux 通过。这取代原分支的前端 clipboard 测试失败记录，
但不代表远端 macOS 或 Windows CI 已运行。
`apps/ora-node/src/tests/review.rs` 另覆盖 R2 结果落库前产生任务提交、R3 创建／删除事件独立确认，
以及 R5 完成写库失败和 Git 观察暂不可用时仍保护替代资源。R1 的定向测试位于 `tests/local_recovery.rs`，
独立 Node 强杀／重启、guardian 丢失及正常停止证据位于 `apps/ora-node/tests/standalone.rs`。
本次不实现 Controller IPC、Controller 接管或旧 Backend 入口切换。静态路径验证不防御恶意 TOCTOU 替换；
外部 Git 不承诺严格恰好执行一次，恢复以持久意图和可验证资源事实为依据。
