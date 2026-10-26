# Revision

[English](revisions.md) | 中文

## 整次工作流交付

SQLite v14 只在原 actor 确认插件及恢复进程结束后记录写入者停止事实；重启补出的终态不补造该证据。缺证据时不做快照或正常释放，已经冻结的字节保留。v13 独立记录 provider 准入，因此原始历史丢失不能把已建立会话变成 `not_started`。

Node 重启后先接上当前绑定，再恢复交付传输。同一所有者重新绑定时，只接纳已登记且全部会话已有持久停止证据的 WorkflowRun 交付；运行、actor、沙盒、代次和持久 Node 身份保持一致，原许可与冻结计划逐字保留。Controller 将这些未完成身份转交 Cloud 核验，不伪报空闲；其他未完成活动仍阻止重新绑定。

`DeliverRevisionSpec.workflow` 明确选择整次运行变体，不能同时携带旧单会话选择器或前序恢复输入。Cloud 给出按执行顺序排列的全部节点尝试和对象键；Node 逐项核对会话终态、actor 和恢复写入者已结束，以及同一 checkout，再在任何推送或上传之前冻结各会话的原始字节和版本化清单。明确的插件启动前失败可记 `not_started`；已建立会话的历史缺失使交付失败，不能缩小清单后报成功。

冻结计划保留完整对象集以供重启重放；上传前再次核对大小和摘要，只为固定键申请校验和绑定授权，最后上传清单。Controller 原样转交全部尝试，只向声明 `workflow_history_delivery` 的 Node 派发，Cloud 独立核验全部对象后登记成果。原 Agent JSONL 形状不变，清单靠工作流变体显式识别，不猜文件扩展名。[整次成果 ADR](https://github.com/ora-space/specs/blob/main/decisions/cloud/workflow/20261025-run-owned-git-delivery-and-history.md) 在完整验收前保持 `approved`。

Agent 会话结束时，Node 把它的 checkout 和历史保存为一个 **Revision**：把最终提交推送到平台 Git Server 上的交付 ref，
对象存储只经 Cloud 签发的授权上传定稿的会话 JSONL。同一个 Issue 之后的运行在 Agent 启动之前从 Git Server 恢复这个
Revision。不生成、不上传也不下载 bundle。相关决策是
[内部快照根决策](https://github.com/ora-space/specs/blob/main/decisions/node/revision/0-internal-snapshot-and-incremental-bundle-upload.md)
（其中的快照提交仍然适用），以及
[推送交付 ref 并从 Git Server 恢复](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261017-push-delivery-ref-and-restore-from-git-server.md)
（`approved`，端到端验收通过后改为 `implemented`），
后者取代了根决策的 bundle 上传和
[会话之前恢复上一个 Revision](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261010-restore-prior-revision-before-session.md)
的 bundle 恢复。Controller 一侧见 [Agent 中继](../controller/agent-relay.zh.md)，授权契约见
[Cloud 契约](../controller/cloud-contract.zh.md#revision-上传校验和)。

Node 同时有 `clone` 和 `git_access` 配置段时声明 `revision_delivery`，因为交付在 clone 的 checkout 中使用 clone 加固过的
Git 环境运行 Git，并以 git-access 身份推送；当且仅当它同时声明 `agent_session` 和 `revision_delivery` 时声明
`revision_restore`。

## 受理

- `controlled_deliver_revision` 是生产入口：它的运行时许可必须与本 Node、incarnation、operation 和 execution 匹配，在
  受理时检查。裸的 `deliver_revision` 只在从未启用运行时控制的 home 中、经私有 IPC 接受。
- 输入指明已结束的会话、它的 clone 执行、基础提交、`refs/ora/revisions/` 下的本地 Revision ref、Git Server 的
  `git_repository_url`、`refs/ora/deliveries/` 下的 `delivery_ref`、`workspace_ref` 和会话历史的对象键。Workspace ref
  必须等于 clone 的 Workspace ref。
- 输入在任何工作之前持久化，交付记为 `Running`。重复相同输入只报告状态；同一身份下的不同输入是协议错误。
- 交付存放在 `revision_deliveries`，唯一的终态事件在 `revision_outbox`，与会话分开，因此会话恢复永远看不到它们。

## 准备

准备只做一次：

1. 会话必须已有终态结果且没有存活的 actor（`failed{session_not_settled}`），它的 checkout 必须仍在交付的基础提交上
   （`failed{checkout_unavailable}`），它的 JSONL 必须存在（`failed{history_unavailable}`）。
2. 定稿的 JSONL 复制到 `<home>/revision-deliveries/<sha256(execution)>/history.jsonl`。
3. 以工作负载用户身份，在 checkout 的 Git 目录中用临时 index 给工作区做快照（`read-tree HEAD`、`add -A`、`write-tree`）。
   如果树等于 `HEAD^{tree}`，最终提交就是 `HEAD`；否则 `commit-tree` 追加一个提交，作者是会话的 Git 身份，提交者是
   `Ora <revision@ora.invalid>`。hook、fsmonitor 和签名都被禁用，真正的 index、工作区文件和分支都不受影响。
   `update-ref` 让本地 Revision ref 指向最终提交。所有运行（手工或定时）在暂存时都排除 `.ora-scheduled-artifacts/`：
   这些文件按对象登记、有自己的可见性与保留策略，快照绝不能把它们发布进交付 ref。因此仓库已经跟踪的同名副本不再随之后的快照
   更新，而历史中已有的副本保持原样。
4. 最终提交等于基础提交，或者（续接的会话）等于上一个 Revision 的最终提交时，声明为 `unchanged`；否则为 `delivered`。
   两种情况都会推送自己的交付 ref。
5. 在第一次网络写入之前，最终提交、会话历史的大小和 SHA-256 以及声明作为冻结计划写入账本。在此之前重启会重新准备；
   之后重启则复用同一份计划。

## 推送与上传

使用 Node git-access 身份的 Git 从不在 checkout 中运行。每次推送使用 `<repository_root>/.ora-transport/` 下的私有传输仓库，
它只借用 checkout 的对象库（拒绝嵌套的 alternates 和共享目录，并禁用 alternate ref 发现）。证书、私钥、CA 和交付执行的
`Ora-Execution-Id` 请求头只交给每条 HTTPS 命令，所以 checkout 的配置无法重定向它，也无法装入 credential helper。

1. 把冻结的提交推送到 `delivery_ref`。新建它，或发现它已指向该提交，都算成功；更新被拒绝是 `failed{push_rejected}`，
   网络或 5xx 失败重试用尽是 `failed{push_failed}`。
2. 尝试把 `workspace_ref` 快进到同一提交，声明 Node 账本上次看到的值。重放算成功；旧值过期、被拒绝或重试用尽只记日志，
   从不让交付失败。
3. 上传会话历史：没有有效授权时（尚未收到、5 秒内过期、被 403 拒绝或因重启丢失），Node 发送带会话历史冻结 SHA-256 的
   `upload_grant_needed`，每 2 秒重复一次，每次新连接时立即再发，交付保持 `Running`。PUT 以流式发送冻结的文件，原样带上
   授权的每个请求头（包括 `If-None-Match: *` 和 `x-amz-checksum-sha256`），从不跟随重定向。2xx 和 412 都算已上传；失败三次
   以 `failed{upload_failed}` 结束交付。授权只存在于内存，从不写日志。

Git Server 的暂时性失败最多尝试三次，间隔 5 秒和 30 秒；401、403 和 429 以外的其他 4xx，以及被拒绝的 ref 更新不重试，以 `push_rejected` 失败。每次推送尝试最长 30 分钟，
与 `network_timeout_ms` 无关，并在一分钟内低于 1 KiB/s 时中止。Cloud 在登记 Revision 之前，于事务外核验交付 ref 和会话历史。

终态 `revision_result`（序号 1）会重放直到被确认；它持久化后冻结的文件即被删除，残留在下次启动时清扫。关闭、Controller
断开和取消从不写入失败；重启后交付从冻结计划继续。

## 定时产物上传

定时运行在 `.ora-scheduled-artifacts/` 下声明产物，Cloud 只保存声明，从不查看 Workspace。因此
`deliver_scheduled_artifacts` 是一个独立执行，在会话结束之后、Workspace 被释放之前运行：

- 生产入口是 `controlled_deliver_scheduled_artifacts`：运行许可必须精确匹配本 Node、incarnation、operation 和
  execution，与其他工作类型一样在受理时校验。输入只命名已结束的会话；Node 从该会话已存储的终态结果读回声明，
  所以重发的执行无法换用另一组文件。
- 没有声明时以 `declaration_unavailable` 失败；checkout 或保留目录不可读时以 `workspace_unavailable` 失败。
- 对象键全部由 Cloud 决定。Node 按声明 index 连同声明的大小和摘要请求授权
  （`scheduled_artifact_grant_needed`），Cloud 按请求顺序为每个产物返回一个授权
  （`scheduled_artifact_grants`）。授权只存在于内存。每个授权只用于它签发的那个声明 index，上传已不再需要的 index
  一律忽略，所以跨越重试或重复请求的应答不会让整条交换失败。
- Cloud 可以永久收回某个上传的授权（`scheduled_artifact_grants_refused`）：运行已离开 `delivering`、冻结窗口内已
  签不出可用授权，或没有配置对象存储。这些状态对该执行不会再改变，所以上传停止请求，按已存下的对象结算，而不是
  一直占着槽位等下去。
- Controller 只把该执行派发给声明 `RevisionDelivery` 的 Node，因为它复用该交付的内存授权中继；这个门控
  是有意为之，而不是缺少独立能力。
- 该上传可以与同一运行的 Revision 交付同时进行，因为会话结束后 Cloud 会派发两者。Node 仍让会话独占所有运行执行、
  让每个交付族自斥，所以第二次 Revision 交付或第二个上传仍以 `resource_conflict` 拒绝。
- 每个文件重新测量后上传到其授权指定的对象键；2xx 和 412 都算已存储。测量出的大小或摘要与声明不一致时，该产物
  在发出任何 PUT 之前就失败，因为 Cloud 已把声明值签进授权。单个产物失败不会阻止其余产物；只要有文件
  已存储，终态就按 delivered 上报这个子集，让 Cloud 逐项登记成功、记录其余拒绝。一个文件都未存下时才以
  `failed{upload_failed}` 结束。空文件使用相同的大小、摘要和授权路径。Controller 转交会话终态时完整保留清单的
  未列明拒绝数、遍历截断标记和 `candidate_limit` 原因。
- 终态 `scheduled_artifacts_result`（序号 1）会重放直到被确认，并共用交付的 outbox。重启后无法续传的尝试以
  `failed{interrupted}` 上报；Cloud 接管这份原终态，按运行冻结的交付截止结算。

## 恢复上一个 Revision

带 `prior_revision`（其身份、最终提交与基础提交、`git_repository_url` 和 `delivery_ref`）的会话，在定位 checkout 并检查
插件之后、把 checkout 交给工作负载用户或启动任何插件之前恢复它。仓库必须是 checkout 当初 clone 所用的仓库；`git_repository_url`
是其他仓库时，会话以 `agent_failed{prior_revision_unavailable}` 结束，不访问 Git Server。

1. 以 Node 身份、使用会话执行的身份，把 `delivery_ref` 拉进一个借用 checkout 对象的私有传输仓库，只以 clone 的基础提交
   协商。拉到的提交必须等于上一个 Revision 的最终提交。
2. 以工作负载用户身份，把对象从传输仓库取入 checkout，并把 Workspace 的本地分支移到上一个 Revision 的最终提交。clone 的
   基础提交、仓库配置和 `refs/remotes/ora/**` 都不变。
3. 如果上一个 Revision 的基础提交不在当前快照的历史中，就在第一轮输入后追加一段固定的英文说明，写明恢复的提交、它的基础
   提交和分支，并建议运行 `ora-git fetch upstream`。

拉取和导入共享一个绝对截止，重试也计算在内。它默认 5 分钟，通过 Node 服务配置
`"restore": { "deadline_ms": 300000 }` 调整；该值必须是正整数，表示有限且可表达的时长，非法配置拒绝服务启动。等待恢复
Git 锁、重试和迟到的 host 交换都不重置截止。交付 ref 不存在、403 或 401 与 429 以外的其他 4xx 时，会话以
`agent_failed{prior_revision_base_unavailable}` 结束：前序 Revision 的 Git 内容永久取不到。401 只说明 Node 自己的
git-access 身份没有通过认证，所以会话以可由之后的运行再续的 `agent_failed{prior_revision_unavailable}` 结束；同一材料重试
不会通过，时限内也不重试。其他拉取、校验或检出失败，以及截止时间到期，同样以 `agent_failed{prior_revision_unavailable}` 结束。
两种情况都不会启动插件进程。恢复期间 Node 停止或收到 `EndSession`，恢复会在下一条 Git 命令之前停下并等待正在运行的那条，
所以会话只在恢复不再写 checkout 之后才结束。恢复期间重启，会话以 `interrupted` 结束。传输仓库只在这次恢复的每条原命令都
取得停止确认之后才删除；所以启动清理会保留每个恢复尚未了结的会话的传输仓库，不会在恢复流程开始前先清空。

每条受管恢复 Git 命令（无论 Node 身份还是工作负载身份）使用自身命令截止和恢复截止中更早的值。到期后不再启动新命令；
进行中的 host 交换同样受这个截止约束，不能因更长的传输超时推迟关闭请求。执行到期后的清理与原 Scope 关闭确认使用独立的
清理预算。迟到的祖先检查不能把已经到期的恢复改判成功。到期不是停止证据。

## 停止确认与半恢复隔离

恢复 Git 在 host 接受命令前，把原 Scope/Run 与会话身份一起持久化，Node 身份的命令（建传输仓库、Git Server 拉取、
`commit_of`）和工作负载身份的导入都在内。首次停止请求先落盘再发 Close；取消只在等待或命令边界生效，不丢弃正在执行的
阻塞工作线程。原 Scope 已观测到 `Closed`，且原 Run 的直接进程已经退出或 host 权威报告 `NotStarted`，才允许
会话终态、传输仓库清理、交付或 checkout 复用。未启动证据必须同时匹配原 Scope/Run 身份、`LaunchFact::NotStarted`、
`DirectProcessState::NotStarted` 和已完成的 cleanup。host 拒绝与持久化的已确认启动事实矛盾的未启动报告，重启后也不例外。
host 查不到记录时返回的 `UnknownIdentity` 仍属未知，不能授权释放。

权威未启动事实来自平台明确的 spawn 失败，或 guardian 接受启动前的取消。后者必须满足当前 host binding 挡住旧请求，
且 Stop 之后不再排入 Start；没有这些 fencing 条件的 guardian UnknownRun 不是证据。当前 Linux guardian 使用 BestEffort，
取消路径报告 `BestEffortComplete`，尽力清理完成不等于 Strong 的静止保证。
依据为[已批准的 NotStarted 扩展](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261021-stop-confirmation-accepts-not-started.md)。

部署配置为 `"restore": { "deadline_ms": 300000, "stop_confirmation_ms": 600000 }`。停止确认上限默认十分钟，
必须为正的有限时长且大于恢复截止；从首次停止请求冻结，重试、重启、重连、配置变更和 Cloud 窗口均不续期。原停止证据未知时
保留身份、输入、待补报原因和 checkout 冲突责任；到期停止本会话的 host 证据轮询，原执行查询显示 `stop_unconfirmed`、原
Scope/Run、首次请求与上限时间、实际纳管保证。到期不发终态，不授权清理或复用。启动交接及后续执行查询只能重试关闭、查询
这些原身份，迟到证据仍在原执行上补报一次；恢复不重做。升级前尚未完成的旧恢复按持久事实判定：已完成的旧终态只作为历史
保留，不拦截后续准入；`last_sequence` 不为零的旧会话同样如此，因为 v9/v10 的恢复在 Agent 启动前同步完成，只有 Agent
线程事件会推进这个计数，而 Controller 的确认只删掉出站事件行、不会把它清零；升级时处于 Accepted、从未启动过 actor 的
会话保持 pending 阶段，因为当时不可能有恢复进程。剩下的 `legacy_unknown` 恰好是升级那一刻可能正在恢复的会话：它们没有
Node 侧出口，由残留回收契约（specs#126）在取得真实证据或运维显式确认后释放。

账本先记录恢复完成，再允许插件启动。带前序输入、恢复未完成且 Agent 从未获准启动的会话，在确认停止后交付只读 HEAD：
只接受 clone 基线或前序最终提交，恒为 `unchanged`，不探查混合工作区，不快照、不写 Revision ref。停止未知时连 HEAD 都
不读。原进程归属未知的传输仓库保留；清理失败单独记录，不改变停止证据。

以上实现[已批准的恢复截止 ADR](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261018-complete-restore-deadline-and-cancel-confirmation.md)
的 Node 侧语义。

## Git Server 访问材料

启用平台 Git Server 的部署增加顶层 `git_access` 配置段，以绝对路径给出 `ca_cert`、`client_cert` 和 `client_key`，这是为
运行时的租户、Workspace 和代次签发的客户端证书，与 WSS 和模型访问的身份分开。入口脚本把这些文件放在只有 root 可访问的
`/run/ora` 中。Node 在启动时校验这些材料，不可用时拒绝启动；Cloud 模式（设置了 `control.target`）的 Node 没有这一段时以
`git_access_required` 拒绝启动。证书只承载身份：Git Server 根据 Cloud 当前状态，按请求头 `Ora-Execution-Id` 中的执行
判定每个请求能读写什么，服务器地址由执行输入给出。同一份材料用于 [clone](repository-clone.zh.md)、交付、恢复和
[ora-git](ora-git.zh.md)。

## 限制

- Agent 插件运行在宿主 Scope 之外，所以“进程 scope 已关闭”指会话结果已写入、会话运行时已停止其插件进程树。
- 交付的 Git Run 由宿主管理，但不作为业务变更记入日志；被中断的准备只是重新准备。
- 仓库配置仍可以选择由 `git add` 运行的 clean 或 smudge 过滤器，它们和 Agent 本身一样以工作负载身份运行。
- 在传输仓库中解析借用的 checkout 对象没有单独的资源限制。
- 旧版 Node 在 Node home 中留下的 `revision-restores/` 目录不会被删除。
- `base_unavailable` 依赖远端对缺失对象的措辞；措辞不同的服务器会被报告为可重试的 `unavailable`。
