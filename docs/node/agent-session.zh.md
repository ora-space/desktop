# Node Agent 会话执行

[English](agent-session.md) | 中文

Node 用 [`ora-agent-runtime`](../agent-runtime.zh.md#运行时-crate-与宿主) 运行
Agent 会话执行：在一次 clone 留下的 checkout 中只启动该执行指定版本的 agent
插件，把每条定型记录先写入会话 JSONL、再交给 Node 账本成为 Thread
事件，并按受理顺序执行会话命令。决策依据见
[agent-runtime 根决策](../../specs/decisions/node/agent-runtime/0-shared-agent-runtime-crate-hosted-by-node.md)
与[协议后续决策](../../specs/decisions/node/protocol/20260928-streamed-thread-events-and-session-commands.md)。

代码位于 `apps/ora-node/src/session`，只在 Linux
构建。账本表、协议消息接线与插件安装不在这里；它们只经下文的
接口与会话执行交互。

## 接口

| trait              | 提供方     | 会话执行用它做什么                                                                           |
| ------------------ | ---------- | -------------------------------------------------------------------------------------------- |
| `SessionLedger`    | Node 账本  | 追加 Thread 事件、写入终态、读取排队命令（按受理顺序的全部）、结算命令                       |
| `CheckoutResolver` | clone 账本 | 由 clone 执行 ID 解析 checkout；会话从不自行拼接路径                                         |
| `PluginCatalog`    | 插件安装   | 取得使用租约，查询指定版本的安装目录                                                         |
| `SessionHost`      | 会话执行   | 由 `AgentSessions` 实现：`start`、`command_arrived`、`recover_interrupted`、`sealed_history` |

`queued_commands`
返回全部排队命令而不是只有队首：轮次进行中必须能看见排在用户轮次之后的
`EndSession`，才能 立即取消当前轮次并丢弃排在它前面的轮次。

## 每个执行一套运行时

每个会话执行组合自己的插件生命周期与 agent 运行时。插件根仍是 Node 数据目录下的
`plugins/`，会话 history 位于同一目录下的
`sessions/`，但这个生命周期只报告、只启动本执行的 agent 插件，并以本执行的 Git
身份启动它。
“只启动指定插件”和“身份只进入该执行的进程树”因此由组合方式保证，而不是由共享实例上的过滤保证。

| 运行时接口           | Node 实现                                                                       |
| -------------------- | ------------------------------------------------------------------------------- |
| `SessionStore`       | `MemorySessionStore`：Node 重启不恢复会话，会话行不需要跨进程保存               |
| `AgentAttach`        | 本执行的插件生命周期，只含一个插件；不登记 Effect consumer（Node 不投影 Skill） |
| `SessionSetup`       | `NoSessionMcp`：MCP 的配置与密钥还没有下发路径                                  |
| `RuntimeEvents`      | 每一行 history 写成一个 Thread 事件；标题与模型目录事件丢弃                     |
| `WorkspaceDirectory` | checkout                                                                        |

Git 身份以 `GIT_AUTHOR_*`/`GIT_COMMITTER_*`
同时设置在插件进程上（插件直接派生的进程继承它）和宿主经
`ora/childprocess/spawn` 为插件派生的进程上（这些进程继承的是 Node
的环境）。Node 不写任何 Git 配置。这些变量叠加在继承的环境之上，Node 环境中的模型凭据和代理设置仍会到达 Agent。

### 独立的工作负载用户

部署以独立用户运行 Git 工作负载（`process.workload_uid`）时，Agent 也以该用户运行：Deno 插件进程以及它请宿主派生的每个进程
在执行前降为工作负载 UID（组与之相同，无附加组、无 capability、`no_new_privs`、umask `077`），无法降权的派生直接失败，
不会以 Node 身份运行。这样 Agent 能在工作负载用户拥有的 checkout 中提交，也读不到 Node 的数据目录。

每个会话获得 `<agent.workload_directory>/<sha256(execution_id)>/`（root，`0711`），其中：

- `package/`：已安装包的硬链接视图，目录为新建的 `0755`（跨文件系统时复制为 `0644`/`0755`；遇到链接或特殊文件则拒绝）。
  插件从视图启动；生命周期仍发现并校验已安装的包。
- `home/`：`0700`，属于工作负载用户。`HOME`、`XDG_CONFIG_HOME`、`XDG_DATA_HOME`、`XDG_STATE_HOME`、`XDG_CACHE_HOME`、
  `DENO_DIR`（在 cache 下）以及 `DENO_NO_UPDATE_CHECK=1` 都指向这里，插件进程与宿主派生的进程一致。

启动前 checkout 会在不跟随链接的前提下交还给工作负载用户，以处理旧版 Node 中以 root 运行的 Agent 写过的 checkout。
插件停止后删除会话目录；服务打开时在结算中断会话之后删除所有遗留的会话目录。准备失败时会话以
`agent_failed{agent_start_failed}` 结束。

## 执行过程

1. `start` 立即返回；会话在后台解析 checkout，找不到时以
   `agent_failed{checkout_unavailable}` 结束。
2. 先取得插件租约，再查询指定版本。目录中没有该版本，或插件根里发现的包不在该版本目录、不是
   agent 时，以 `agent_failed{agent_plugin_unavailable}`
   结束，此时没有启动任何插件进程。租约持有到插件进程整树退出之后。
3. 等待 agent 连接就绪（有上限，超时或监管放弃为
   `agent_failed{agent_unavailable}`），以执行 ID 作为 Ora Session ID
   创建会话（失败为 `agent_failed{agent_start_failed}`），然后发送
   `initial_turn`。
4. 轮次进行中定型的每一行 history 都以当前轮次的 `turn_id` 成为 Thread
   事件；用户消息在 JSONL 中也以 ACP `messageId` 带着同一个标识。超过 256 KiB
   的记录在 Thread 中只保留 `at`、`seq`、`type` 并标注 `truncated`，JSONL
   保留原文。
5. `command_arrived` 唤醒会话读取队列。`SubmitUserTurn`
   在当前轮次结束后按受理顺序执行，开始执行时结算为
   `executed`；重复唤醒不会重复执行。`EndSession` 把排在它之前的用户轮次结算为
   `discarded`，停止会话（取消 进行中的轮次并记录 `TurnEnded{cancelled}`，支持时
   `session/close`），再结算自身为 `executed`。
6. 结束时：停止会话，释放运行时（连接监管随之停止重连），停止插件并等待整树退出，释放租约；把仍排队的命令结算
   为
   `discarded`，写入终态后再移除存活记录；服务停止会等待该写入。交付看到会话结束时
   history 已不再被写入。

Agent 轮次失败或超时只记录 `TurnEnded`，会话保持。轮次无法被接纳（agent
无法连接）时以 `agent_failed{agent_unavailable}` 结束。

## 记录顺序与崩溃

运行时逐行写 history，每行写入文件后、写下一行之前同步调用
`record_settled`，Node 在其中完成 `append_thread_event`。因此 Thread
中的每条记录都在 JSONL 里且顺序一致；两次写入之间崩溃时，JSONL 最多比 Thread
多一行。

`append_thread_event` 一旦失败，镜像永久停止（之后的行不再进入 Thread，避免
Thread 出现无法解释的空洞），会话以 `agent_failed{thread_unavailable}` 结束。

Node 重启后，没有终态的会话执行由 `recover_interrupted` 以 `interrupted`
结束：history 唯一的写入者已随旧进程
结束，文件内容就是最终内容。`sealed_history` 在会话仍在本进程运行时返回
`history_unavailable`，否则返回 `ora-history` 规定路径下的 JSONL。

## 已知差距

插件进程与 Agent CLI 由 Node 像 Desktop
一样直接启动（进程组整树终止），会话结束与 Node 正常停止时整树清理；
它们还没有纳入 host/guardian 的执行进程 scope，因为 guardian
目前不提供插件所需的 stdin 与协议流。Node 崩溃时， 插件会因 stdio
关闭而退出，但忽略这一点的后代进程不会被回收，直到新的 process I/O
决策覆盖插件。

## 测试

`apps/ora-node/tests/agent_session.rs` 用内存账本、固定 checkout
与带租约计数的插件目录，通过 `SessionHost` 驱动真实的 echo agent
插件进程（`ora-node-echo-agent`，与 E2E 的 `fake-agent` 一样顶替
`deno`）。覆盖记录顺序
与轮次归属、命令排队、结束时的取消与丢弃、崩溃窗口与中断恢复、插件版本不符、Git
身份与超大记录。夹具只用于 测试；Node 镜像只复制 `ora-node`。

持久化适配见[会话账本](session-ledger.zh.md)。

## 生产服务

在服务配置的 `node`、`process`、`clone`、`control` 旁添加：

```json
"agent": { "deno_path": "/usr/local/bin/deno", "ready_timeout_ms": 30000 }
```

有工作负载用户时再添加 `"workload_directory": "/var/lib/ora/agent"`。它在且仅在设置了 `process.workload_uid` 时必需，
必须是已存在的绝对 UTF-8 路径，属于 Node 身份且其他人不可写，并且不能与 Node home、进程宿主目录或 clone 根目录重叠。
Node 从不创建它，由沙箱入口脚本创建。

Deno 路径必须为绝对路径，等待就绪的超时必须为正数。配置后握手声明 `AgentSession`；未配置时拒绝新会话工作，
但启动时仍结算已有未终态会话。部署负责提供 Deno 和已安装插件包。服务为 `AgentSessions` 注入持久化账本、
checkout 解析器以及插件执行所用的同一个 `PluginInstaller::catalog()`。插件恢复先于任何会话启动。

Cloud 受控启动使用 `ControlledStartAgentSession { binding, command }`，需要 RuntimeControl 和 AgentSession
能力。worker 检查 Controller 归属、精确执行身份与运行许可，持久化后在启动前再次检查。裸启动仅允许未启用
控制隔离的本地 IPC；重复启动只返回状态，不重建 actor。新命令要求原 control scope 的当前绑定有效，关闭、
过期或变更 scope 都不能授权新输入。已终态会话返回 `SessionCommandRejected{session_ended}`。
受理回复发送后才调用 `command_arrived` 唤醒 actor。

每条连接为每个执行最多预留 256 个事件位置，包含已排队等待 socket 写出的事件。按最后预留序号分页读账本，
一个执行窗口满不会挡住其他执行或控制回复；只有持久化成功的精确 ACK 才释放位置。重连丢弃连接内游标，
从磁盘中最小未确认序号开始重放，保留原始内容与身份。发送窗口满时 Agent 仍继续记录。

控制监听入口开放前，所有未终态会话先结算为 `interrupted`，不恢复 Agent。正常停止取消活跃对话、等待插件
清理、提交终态，再释放 Node 数据库租约。命令 Executed 写入失败时不发送该轮 prompt；actor 异常退出或
终态持久化失败会停止服务受理，保留可恢复的持久责任。

已批准的 agent-runtime D2 落地差异仍适用：插件 stdio／进程组支持正常停止清理，但 SIGKILL 后忽略 stdio
关闭的后代尚未纳入 host/guardian。崩溃测试证明 echo 插件退出及中断重放，不代表任意孤儿进程均被回收；
进程 I/O 纳管仍是后续依赖。

`tests/standalone/agent_sessions.rs` 及子模块用生产 Node 可执行程序、真实 clone 和 echo 插件验证命令去重、
历史记录一致、共享安装租约、runtime 关闭、正常停止、强杀恢复、窗口饱和与精确重放。Cloud 工作项的
Controller 中继不属于本次变更。

## 平台模型访问

`AgentSessionSpec.model_binding_id` 是可选的 Cloud 不透明引用。部署在 `agent.model_proxy` 中配置
`gateway_url`、`ca_cert`、`client_cert` 和 `client_key`，证书路径必须为绝对路径。专用客户端认证证书
绑定 runtime 的租户、Workspace 和代次，与 Node WSS 服务端证书分开发放。服务在声明 `ModelProxy`
能力前验证这些材料；Controller 注册、重发和 Node 本地受理均要求带模型绑定的启动具备此能力。

Node 向 `POST /internal/v1/model-grants` 仅提交绑定和会话执行 ID。平台返回冻结的协议、模型及
内存中的临时令牌，上游 API Key 从不进入 Node。OpenCode 使用单一 `ora-model` provider，分别使用
OpenAI 兼容或 Anthropic SDK。配置以环境变量占位引用 `ORA_MODEL_ACCESS_TOKEN`，原样保留含斜线的模型 ID，
访问平台 HTTPS 数据端点，通过 `NODE_EXTRA_CA_CERTS` 信任公开 CA。使用独立工作负载身份时，HOME、XDG
及 `OPENCODE_CONFIG_DIR` 指向该用户拥有的会话 home；Node 只在其旁发布 root 拥有、`0644` 的已验证公开
CA 文件，使 CLI 能验证代理而无法读取管理目录或客户端私钥。共享身份部署沿用 Node `model-runtime/`
下独立的会话临时目录。两种目录均与 checkout 及 Revision 隔离；临时令牌继续使用环境引用，不随公开
CA 写入文件。Git 身份仍来自运行。

续期在过期前延长同一授权，不更换令牌。续期被拒绝时取消对话；正常结束先停止续期、撤销授权、
删除临时状态，再写入终态。任务取消也中止续期并尽力撤销。Node 崩溃后不恢复会话，Cloud 终态恢复
及授权过期会撤销其权限。不带模型绑定的 Echo 会话保持既有环境，且不请求模型授权。
授权创建或续期返回已验证的 `model_session_ending` 时，Node 停止模型权限，最多等待 30 秒接收
已经持久化的 EndSession 命令，以保留其明确的结束原因。普通撤销和服务不可用不会进入等待。
Agent 尚未启动就结束时，封存空的 Ora 历史文件，支持交付未改动的 Revision，不伪造 provider 会话。
