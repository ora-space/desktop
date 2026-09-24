# minicloud 本机运行时

[English](runtime.md) | 中文

## 一键开发环境

在仓库根运行 `task run:minicloud`。脚本安装前端依赖、构建 debug 程序，然后依次启动
host、`ora-controller --single-node`（由它自己启动 Node）和 Vite，打开 `http://127.0.0.1:5174`。
需要 Linux、Deno、Cargo、Node.js、Git 和 `setsid`，不需要 root。

所有开发配置及运行数据都在 `~/.ora/minicloud/<digest>/`，其中 `<digest>` 由 checkout 路径推导，
`workspace` 文件记录该路径。因此每个 checkout 各有独立状态；被其他 checkout 占用的目录会被拒绝。
启动器不向仓库内写入任何内容：

- `config/node.json`、`controller.json`、`client.json`：Node 与 Controller 部署配置（Controller 文件的
  `single_node` 段指向 `node.json`），以及 Vite 端口和启动器经命令行传给 Controller 的回环端口
  `controllerPort`（默认 4820）。已移除的 minicloud server 留下的旧 `server.json` 会被忽略。
- `config/clone.gitconfig`：非交互 Git 配置；默认只适用于无需凭据的 HTTPS 仓库，私有仓库凭据需自行配置。
- `node/`、`controller/`：数据库与 Node IPC；`p/`：host／guardian 状态（短名称为 Unix socket 长度留空间）。
- `repositories/`：clone 目录；`home/`：工作负载 HOME；`bin/`：可识别归属的版本化 guardian；`vite/`：Vite 缓存。

重复运行保留配置、数据库及 checkout，不覆盖用户编辑，不在恢复失败时清库重建。日志输出到终端；
依赖和编译产物仍使用仓库标准 `node_modules`／`target`，不复制到数据目录。
仅初始化可运行 `deno run -A scripts/run-minicloud.ts --init-only`；已构建时可用 `--no-build` 跳过安装和编译。
修改端口后重新运行；由脚本拥有的数据目录路径不能改到其他部署。

Ctrl+C 或组件异常退出会先停止 Vite，再停止 Controller——由它停止自己启动的 Node 并等待受管 Git 收尾，
随后停止 host 和本目录专用 guardian。Node 与 Controller 同属一个进程组，因此即使 Controller 崩溃，
启动器按进程组停止时仍能到达 Node。停止超时会报告并升级信号；不保证终止逃逸的工作负载后代。中断的 clone 可能保持待恢复／未知，
不会自动新建执行或删除残留。脚本使用独占锁，拒绝同一数据目录的第二个启动器。

debug 构建跳过受信路径的 Unix 权限位检查，允许组可写的项目目录，不修改已有目录权限。
所有者、符号链接、硬链接、类型、目录隔离及数据库独占校验仍生效；release 构建仍强制权限位检查。
状态目录以家目录而非 checkout 为基准，因为 Unix socket 路径上限为 108 字节；异常长的真实家目录路径会被拒绝，
不会被截短，也不能用符号链接绕过。

## 云端持久模式

`task run:minicloud -- --cloud` 同样启动 host、`ora-controller --single-node` 和 Node，但 Controller 以
[云端持久模式](../controller/local-runtime.zh.md#独立可执行入口)运行：所有持久事实由 Cloud 持有，Controller
只主动调用 Cloud，因此不打开 SQLite 数据库、不开监听，也不启动 minicloud 前端。请求改由 Cloud 的带租户
clone API 进入。

Cloud 需另行启动，且必须来自提供 clone API、不要求 Controller 认证的版本。在 Cloud 仓库中把
`config.toml.template` 复制为 `config.toml` 并执行一次 `task setup`（PostgreSQL DSN、Gateway 密钥、
`.local/dev.env`、迁移），然后分别执行 `task run` 启动 Cloud server（HTTP `:8080`、Controller gRPC `:8082`）
与 `task run:gateway` 启动认证 Gateway（`:8081`），两者见 Cloud 仓库的 `docs/gateway.md`。
当前阶段 Controller 不出示凭据，只以 `controller_id` 声明自身，Cloud 把它记为租约持有者。

状态位于 `~/.ora/cloud/<digest>/`，与本地模式的目录分开：Node 的持久记录只属于一个持久权威，因此 SQLite
接受的工作绝不会报给 Cloud，clone 目标目录也不会共用。目录结构与本地模式相同，只是没有 `client.json` 和
`vite/`；`config/controller.json` 选择 `persistence: cloud`，包含 Cloud 的 gRPC 地址与领取间隔，
并且没有 `api` 段。Cloud 在别处时修改该文件即可。持久模式与启动模式不一致的 `controller.json`
会被启动器拒绝。

Cloud 的 gRPC 地址不可达时，启动只提示一次并继续，因为 Controller 会持续重试 Cloud。Controller 没有可探测的端口，
所以"就绪"只表示它启动两秒后仍在运行，不表示已取得 Cloud 租约；之后任何组件退出都会像本地模式一样停止全部组件。
clone 与其他 Cloud 客户端的请求一样经 Gateway 提交：使用其开发登录得到的会话，以及该用户的租户；
这一侧不持有任何密钥或 token。Gateway 在 `:8081`、默认公开来源为 `http://localhost:5173`，写请求须以
`Origin` 声明该来源：

```bash
G=http://localhost:8081 O=http://localhost:5173 JAR=$(mktemp)
# 开发登录：发起登录、提交身份表单、跟随回调拿到会话 Cookie。
AUTH=$(curl -s -c "$JAR" -H "Origin: $O" -H 'content-type: application/json' -d '{"provider":"dev"}' "$G/auth/login" | sed -E 's/.*"authorizationUrl":"([^"]*)".*/\1/; s/\\u0026/\&/g')
CALLBACK=$(curl -s -o /dev/null -w '%{redirect_url}' -b "$JAR" -c "$JAR" -H "Origin: $O" -d "${AUTH#*\?}" -d source=dev -d subject=minicloud "$G/auth/dev/authorize")
curl -s -o /dev/null -b "$JAR" -c "$JAR" "${CALLBACK/#$O/$G}"
# 创建租户按用户与幂等键去重，重复执行返回同一个租户。
TID=$(curl -s -b "$JAR" -H "Origin: $O" -H 'content-type: application/json' -H 'Idempotency-Key: minicloud' -d '{"name":"minicloud","slug":"minicloud"}' "$G/api/v1/tenants" | sed -E 's/.*"tenant":\{"id":"([^"]*)".*/\1/')
curl -s -b "$JAR" -H "Origin: $O" -H 'content-type: application/json' -H 'Idempotency-Key: r1' -d '{"requestId":"r1","repository":"https://github.com/octocat/Hello-World","branch":"master"}' "$G/api/v1/tenants/$TID/clones"
```

`curl -s -b "$JAR" "$G/api/v1/tenants/$TID/clones"` 列出操作，`.../clones/{operationId}` 读取单个操作；
`state` 从 `pending` 变为 `succeeded`（`path`、`commit`）或 `failed`（`reason`，可选 `retainedPath`）。
以相同 `requestId` 重复提交会返回原操作，不会再次 clone。Controller 停机期间被接受的工作保持 pending，直到被领取。
若上次运行的 Controller 是被强杀而非正常停止，旧租约没有释放，工作要等 Cloud 的 30 秒租约过期后才开始。

## 手动部署

API 由 `ora-controller` 可执行程序本身提供，没有独立的 minicloud server。先启动 host／guardian，
再按既有[部署配置](../node/repository-clone.zh.md)启动 Node，或用 `--single-node` 让 Controller 托管它。
然后运行 `ora-controller --config /absolute/path/controller.json --transport tcp --host 127.0.0.1
--port 4820 [--single-node]`；配置文件、参数与托管规则见
[Controller 运行时](../controller/local-runtime.zh.md#独立可执行入口)。

Node 配置的归属必须匹配 ControllerId。启动 minicloud 前，停止使用同一状态目录的独立 Controller。
各状态目录继续显式注入并拒绝重叠；非回环监听或未配置的目标 Node 会在打开 Controller 状态前拒绝。
这是非生产应用，不增加认证或额外安全体系。

## HTTP 接口

先运行 `deno install`，再从仓库根执行 `deno task --filter @ora/minicloud-client dev`，
打开 `http://127.0.0.1:5174`。Vite 将 `/api` 代理到 `http://127.0.0.1:4820`；
不同 Controller 端口可通过 `MINICLOUD_SERVER_URL` 配置。
页面使用共享 shadcn 组件及 React 19，轮询 Controller 操作。
提交前把未确认请求写入当前标签页 session storage；回复丢失或刷新后，“重试原请求”复用原身份与输入。
这不是操作历史数据库。关闭页面终止 HTTP 和轮询，不取消 Node 执行。

- `POST /api/clones`：`{ "requestId": "stable-client-id", "repository": "https://host/repo.git", "branch": "main" }`。
  持久接受后返回 HTTP 202，包含 `requestId`、`operationId`、`executionId`。
- `GET /api/clones`：按接受顺序倒序列出操作，Node 离线时仍能读取待协调记录。
- `GET /api/clones/{executionId}`：读取单个操作；404 表示没有该身份的接受记录。

状态为 `pending`、`succeeded`（Node 路径及 commit）、`failed`（明确原因及残留路径）。
pending 不声明 Git 是否正在运行。HTTP 400 表示输入无效，409 表示身份／输入冲突，503 表示暂时不可用；
HTTP 错误不是 clone 终态。浏览器 DTO 从 `ora-contracts::controller_api` 生成，与 Node 消息协议分开；
这套 JSON 接口是过渡契约，浏览器改经 Ora Cloud 接入后退役。

未确认接受结果时使用相同请求身份和输入重传。关闭浏览器或停止 Controller 不取消 Node 执行；
重启使用原状态目录恢复事实。不另建 minicloud 任务数据库、不提供清理命令或自动新执行重试。

真实 HTTP 测试（`cargo test -p ora-controller`）覆盖接受、冲突、无效输入、离线列表、不存在的执行身份、
独占、组合参数拒绝、Unix socket 传输及正常重启。
下层 Controller 强杀测试与 minicloud 自身端到端证据分别记录。

前端检查：`deno task --filter @ora/minicloud-client lint`、`test`、`build`。
`task test:minicloud` 另验证真实 HTTP 及 Vite proxy → 独立 `ora-controller` → Node → HTTPS Git，
包含 Controller SIGKILL／重启和唯一变更 Run；要求 Linux 且已安装前端依赖。
Vite 用例在纯 Rust CI 中显式跳过，通过专项任务执行。
真实链路还通过 SQLite writer lock 注入接受写失败，验证 HTTP 503 且没有接受记录，释放后仍只有一个 Run。
真实 TCP 代理截断接受响应 body，Controller 重启后重传仍保留原执行；clone 期间正常关闭 Controller，
固定身份的 Git 进程仍存活，最终返回同一结果且只有一个 Run。入口测试验证重叠目录拒绝、
未知文件保留，并复用独立 Controller 所有者接受的记录。`--single-node` 组合另有专项验证：强杀 Controller
后其 Node 与 Git 继续运行，endpoint 仍活跃时第二个托管 Controller 被拒绝，替换的 Controller 接管重放的
结果，正常停止时托管 Node 被收回。
DOM 测试验证页面行为、轮询自动恢复及刷新身份恢复；完整浏览器引擎交互验收明确不在本次范围内。
