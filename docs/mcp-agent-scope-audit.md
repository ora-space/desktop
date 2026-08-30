# MCP Agent 配置范围审计

> 状态：P1 已于 2026-08-30 确认；本文保留分析与访谈证据，正式边界见 [ADR-0015](./adr/0015-bound-the-first-mcp-agent-loop-to-an-ora-owned-file.md)。  
> 日期：2026-08-30

## 审计目的

本文回答两个问题：当前设计相对最初目标扩大了多少，以及哪些设计是首期闭环真正需要的。Q85–Q93 的选项与依赖仍保留为决策轨迹；用户最终选择 P1，本文不再把这些边界视为开放问题。

## 已确认的 P1 边界

用户确认：`85.B / 86.B / 87.A / 88.A / 89.A / 90.B / 91.A / 92.A / 93.A`。

这意味着一期：

1. 保留原始“写入 Agent 工作目录物理配置文件”的验收条件，并通过现有 Effect worker 收敛；
2. 使用独立 typed MCP Surface，不统一 Skill/MCP generation，也不引入 AgentTarget aggregate；
3. 复用现有 Agent Effect wait/restart，只保证同一 worker claim batch 内每个共享 Agent 最多 activation 一次；
4. 通过运行时 `agent_mcp_v1` surface 与受限单文件 renderer 扩展 OpenCode Agent，不建立完整静态 Contract v2；
5. 只在 MCP Application State 为 Ready 后开始首个闭环对话，不建设全 Session 入口 admission gate；
6. 只创建完整 Ora-owned `.opencode/opencode.jsonc`，目标已存在即 fail closed；ownership 使用数据库 ledger + 同文件头 marker，不创建独立 sidecar；
7. MCP key 由 canonical Plugin ID 派生，并扫描 Workspace 可见配置层；无法观察的全局/managed 层碰撞作为一期已知限制；
8. Settings 完整但没有 MCP-capable Agent 时显示 `WaitingForAgent`，不能显示 `Ready`。

P1 不是“只改一个文件”的小改动：它仍然跨 installer、configuration resolution、Effect、Agent runtime、OpenCode adapter、Settings 状态与 E2E。但它把每个模块限制为一个已证明需要的 profile，明确排除用户 JSONC 合并、独立 sidecar、stdio、全局 activation cohort、全入口 gate、完整 retirement 与第二种 Agent。

## 最初的最小闭环

首期目标可以收敛为一条可验证的纵向链路：

1. Ora 能从现有 Marketplace 下载、校验并安装 OpenCode Agent 插件和 Tavily MCP 插件。
2. 用户在现有 MCP Settings 页面录入 Tavily API Key；首期允许以普通字符串明文保存。
3. Ora 参考 Skills 的配置方式，在某个明确时机或动作中，把已经 Ready 的 Tavily MCP 配置元数据写入 Agent 对应工作目录的配置文件。
4. 新建一次真实 Agent 会话，并在该会话中成功调用一次 Tavily 工具。

因此，“产生物理配置文件”是原始验收条件，不是本轮设计自行增加的范围。这个闭环仍然没有天然要求先建立完整的统一 Skill/MCP generation、多 Agent Target 编排、stdio 生命周期和聚合 UI 平台，但只要需要安全修改一个可能由用户共同维护的 Agent-native 文件，就必须正视所有权、并发编辑和崩溃中断问题。

## 已核实的现状

### Ora 当前实现

- Effect 状态与文件 Surface 仍然以 Skill 为中心；`WorkspaceEffectSpec` 只有 Skills，Agent 插件目前也只接受 `skill_directory.v1`。
- Effect Worker 当前按 Surface claim、应用和重启，并没有 AgentTarget 聚合、批量重启或跨 Surface 原子提交语义。
- 数据库中的 Effect 表结构有一定通用性，但 Rust 领域对象和 Repository API 仍然是 Skill 专用。复用数据库表不等于 MCP 已经具备可直接复用的领域模型。
- Agent Runtime 当前是应用级共享连接。Workspace Session 会复用这个连接，而不是每个 Workspace 启动一份 Agent 进程。
- Plugin SDK 的 child-process 能力已经支持传入环境变量，因此 OpenCode 子进程配置不必经过 Workspace 文件才能生效。

### Marketplace 与 OpenCode Agent

- Marketplace `main` 当前已经指向 OpenCode Agent [`0.3.0`](https://github.com/ora-space/marketplace/blob/main/registry/o/ora-space.opencode/orax.toml)，提供 macOS arm64、Linux x64 和 Windows x64 targeted artifacts；发布产物内置 OpenCode CLI `1.18.25`。
- 因此，“把 0.3.0 上架 Marketplace”已经完成，不属于本功能范围。保留不可变的 `0.3.0`，将 MCP 协议能力放入后续 `0.4.0`，再在 Host 支持发布后推进 Marketplace，属于正常兼容演进。
- OpenCode Agent 当前一个插件进程对应一个 Agent，已经实现 Skills Effect，但没有 MCP 配置方法。

Ora 的安装链已经覆盖绝大多数原始验收：Marketplace manifest 解析和 host target 选择、流式下载、SHA-256 校验、下载大小限制、安全 archive extraction、同父目录 staging，以及验证完成后的目录 rename 都已经存在。MCP package 也已经有静态 `assets/config.json` 校验。因此首期不需要建立新的下载器、签名系统或安装状态机。

已确认的窄缺口位于 marketplace install 的提交边界：`Installer::install_package` 使用 Marketplace manifest 验证 staging 内容。OpenCode 这类 targeted release 会读取包内 `orax.toml`，但只核对 artifact target；Tavily [`0.1.0`](https://github.com/ora-space/marketplace/blob/main/registry/o/ora-space.tavily-search/orax.toml) 这类 universal release 在 rename 前完全不读取包内 Manifest。两条路径都没有核对包内 namespace/name/version/kind。若包内 manifest 缺失或身份不同，包会先落入 Marketplace manifest 指定的目录，随后 backend 的 `sync_plugin_skills` / discovery 才报不可发现；残留目录还会使下一次安装得到 `AlreadyInstalled`。首期应在 staging 内始终解析 installed manifest，核对四项身份并使用该 manifest 做静态 package validation，然后才 rename。这个改动是 installer 的局部 fail-before-commit 加固，不是 MCP Effect 或 lifecycle 范围。

### Tavily 与 OpenCode 配置能力

- Marketplace 中 Tavily 首个目标是远程 HTTP MCP，配置核心是 URL 与 Bearer API Key；首期不要求 stdio 生命周期管理。
- OpenCode `1.18.25` 支持 `OPENCODE_CONFIG_CONTENT`，并且该配置在项目配置之后合并。
- OpenCode 配置支持 `{env:VARIABLE}` 引用；Ora 可以只把环境变量名写入配置内容，把实际 API Key 放入子进程环境。
- 这意味着 Ora 可以在不修改 Workspace 文件的情况下，为共享 OpenCode Agent 注入一份运行时 MCP 配置。相同 MCP key 会在运行时覆盖项目配置，但不会改写用户文件；移除注入后，用户原配置会自然恢复。

Plugin Configuration 也已经具备首期解析所需的大部分输入：MCP `assets/config.json` 会被严格编译为 HTTP/stdio exclusive enum，Setting reference、prefix/suffix、Header name、HTTPS URL 和环境变量名都有静态校验；Settings `store.json` 已提供单调 revision、compare-and-save、默认值投影、完整性状态和原子写。源码甚至明确把 `ResolvedMcp` 标为“later, separate step”，说明 seam 已经预留但尚未实现。

因此，首期缺的是一个局部纯 resolver：输入 exact installed descriptor、对应 revision 的 effective values，以及首期允许的 runtime context，输出完整 `ResolvedMcp` 或 `NeedsConfiguration/UnsupportedTransport`。它不需要新建 Settings 表、修改现有编辑器协议或把 SQLite 引入 `ora-plugin-config`。现有 compiler 支持 stdio 并不意味着首期必须 materialize stdio；OpenCode + Tavily profile 可以在 resolver/adapter capability 边界明确只接受 HTTP，保留已经存在的静态类型而不扩展运行时生命周期。

当前 `PluginApi::save_configuration` 和 `reset_configuration` 在成功写入新 revision 后直接返回，没有 enqueue Effect propagation。把一次 coalescing MCP source refresh 接到这两个成功边界，以及安装/更新完成边界，是首期必要的 integration glue；它不需要把 `store.json` 写入和 Effect enqueue 伪装成一个跨文件/数据库事务。进程在两者之间退出时，启动同步必须从当前 installed descriptors 与 `store.json` revision 重建期望状态，这一补漏规则比通用 source-event 平台更窄。

## 首期真正需要补齐的能力

在不改变原始物理文件验收、并保持与 Skills 相同的 Effect 收敛时机时，以下能力直接服务于最小闭环：

1. 在现有安装链上补齐 staging 内外层 Manifest 的 namespace/name/version/kind 一致性校验；复用既有下载、SHA-256、安全解压和原子提交能力，不重构 installer。
2. 从已安装且 Ready 的 MCP 插件配置中解析出规范化的 `ResolvedMcp`。
3. 给 Agent 插件增加一个完整集合、幂等的 MCP 配置入口，由现有 Effect 收敛流程调用，而不是把写文件塞进 `session/new`。
4. OpenCode Adapter 将完整集合渲染成 Ora-owned Workspace JSONC 文件；文件只保存环境变量引用，API Key 由受信 Agent 进程注入 OpenCode 子进程环境。
5. MCP 安装或 Settings revision 变化后，为现有本地 Workspace enqueue MCP surfaces；worker 应用成功后按共享 Agent 合并一次 wait/restart，并在 activation 成功后标记 Ready。
6. 用真实 Tavily API Key 完成一次端到端工具调用验证。

其中第 5 项需要每个 Workspace × Agent 文件 surface 的期望摘要、已应用摘要和 consumer-ready 状态，但 surface 本身已经是调度与状态单元，不要求再派生一个持久 AgentTarget aggregate，也不要求统一 Skill+MCP generation。若用户明确放弃物理文件验收，才可以把第 3–5 项替换为后文的 `OPENCODE_CONFIG_CONTENT` 运行时方案。

## 当前设计的扩大链条

当前 ADR 集合把上述纵向闭环逐步扩展成了一个通用配置平台：

| 设计项                                     | 对首期是否必要                                | 引入的主要成本                                                                               |
| ------------------------------------------ | --------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Skill 与 MCP 统一 generation               | 否                                            | 改动现有稳定 Skill 状态机与兼容路径                                                          |
| 派生 AgentTarget、聚合 Ready/Applied       | 否                                            | 分组、投影、部分成功和恢复语义                                                               |
| 所有 Session admission 统一屏障            | 只需覆盖共享 Agent 连接的配置代际             | 扩展到恢复、懒加载和多入口并发控制                                                           |
| 顺带修复 Skill 首会话竞态                  | 与 Tavily 闭环无直接关系                      | 将独立历史问题纳入交付关键路径                                                               |
| 同时支持 HTTP 与 stdio MCP                 | 否，Tavily 是 HTTP                            | 进程生命周期、Workspace 上下文和平台差异                                                     |
| Workspace 配置文件                         | **原始要求，首期需要**                        | 必须决定准确路径、格式、应用时机和 Agent 加载边界                                            |
| sidecar、数据库 ledger                     | 是否全部需要取决于所有权保证                  | 多文件一致性、崩溃恢复和所有权判定；不是“写文件”四个字自动要求所有机制齐备                   |
| 完整 CAS、Plan/Apply/Observe、复合 journal | 独立 sidecar 或用户文件合并方案才需要完整形态 | 新事务协议、故障注入和恢复测试矩阵；Ora-owned 单文件仍需要较窄的 Prepared + fingerprint 恢复 |
| 用户 JSONC 保真编辑                        | Q91.A 下不需要                                | 编辑器级语法保真；仅在必须兼容既有目标文件时进入首期                                         |
| Git local exclude 与本地路径安全           | 物理文件方案需要                              | 可以作为发布文件前的幂等前置条件；首期不需要远程 Workspace                                   |
| 卸载、退役、更新保留策略                   | 不是首次安装与调用的验收条件                  | 新的生命周期状态和兼容规则                                                                   |
| 聚合 UI、细粒度错误分类、线上 CI smoke     | 可以后置                                      | 多层产品与发布基础设施改动                                                                   |

这些能力并非没有价值，但它们解决的是后续通用化和运维问题。把它们全部设为首期前置条件，会把一个可单独交付的 OpenCode + Tavily 集成变成跨数据库、Effect Worker、Agent Runtime、文件系统、前端和发布流水线的多阶段平台重构。

## 扩大来自哪些组合约束

当前复杂度不是每一项都能独立删除。下面三条组合链说明哪些平台机制是由此前产品选择推导出来的：

```text
物理 Workspace 文件（原始要求）
  + 全局自动启用（访谈选择）
  + 对所有既有 Workspace 立即物化（ADR 扩展）
  + 一个共享 Agent 进程（现有架构）
  + 不打断活跃 turn（访谈选择）
  → 跨 Workspace quiesce、批量应用、一次重启、部分成功与聚合 readiness

数据库 ledger
  + Workspace sidecar 双重所有权（41.A）
  + fail-closed 且崩溃后自动恢复
  → Prepared intent、Plan/Apply/Observe、artifact journal 与 CAS

任何 Session 路径都不能使用旧 generation
  + warm / load / workflow 已经存在
  → 中央 admission barrier、活动状态与 replacement 传播
```

因此，“是否过度设计”不能只看最后出现的机制：在保留每条链左侧全部约束时，右侧机制大多有真实理由。范围收缩必须明确放松至少一个前提。例如把“全局自动启用”解释成“任何 Workspace 使用时都会自动获得”，而不是“安装后立即为数据库里的全部 Workspace 写文件”，就可以把首期从跨 Workspace 批量编排收回到按需 surface 收敛，同时仍然没有 Workspace opt-out UI。

反过来，如果坚持 eager-all-Workspaces、双重所有权自动恢复和全入口 stale-generation 禁止，那么当前方案虽然很大，却不能简单定性为无根据的过度设计；它是一项有意的平台投资，问题在于它已经超出原始 OpenCode + Tavily 闭环，应拆成独立里程碑并重新估算，而不是继续称为“第一阶段最小闭环”。

## 两处关键设计偏移

### 1. “参考 Skills”被解释成了“复制 Skills 的持久化形态”

Skills 的目录内容和 marker 可以放在同一暂存目录中，再通过一次目录 rename 原子提交。OpenCode 配置文件和独立 sidecar 是两个不同文件，无法天然获得同样的原子性。

所以二者可以共享“声明期望状态、幂等应用、失败可恢复”的设计原则，但物理实现并不等价。正是额外 sidecar 导致了 journal、CAS、恢复投影和所有权协议的连锁扩张。

### 2. Workspace 文件 Surface 被升级成了通用 AgentTarget 编排

当前首期选择是“所有 Ready MCP 全局自动启用”，目标 Tavily 的配置内容不依赖 Workspace；但原始验收要求把文件写到 Agent 对应工作目录。因此 Workspace 维度不是虚构的：不同 Workspace 有不同目标路径、占用冲突、Git 状态和收敛结果，一个 `Workspace × Agent` 文件 Surface 是合理的最小持久单元。

偏移发生在这个 Surface 又被升级为派生 AgentTarget aggregate，并要求它聚合 Skill/MCP generation、所有 Session 入口、完整生命周期和跨 Surface 原子 readiness。首期可以让各 Surface 独立应用，再在一个 worker batch 内按共享 Agent 临时分组并只 restart 一次；这不需要额外的持久 Target 投影。以后支持 stdio MCP、Workspace 命令、相对路径或每 Workspace 策略时，再证明是否需要更强 Target 模型。

## 文档内部的设计漂移

目前研究文档、Spec、CONTEXT 与 ADR 之间存在需要在范围确认后统一的差异：

- 旧 Spec 仍描述 Workspace 选择、SecretRef 和在 `start_agent` 前配置；后续访谈选择的是全局自动启用、普通字符串和由 Worker/会话门槛协调。
- 当前 Marketplace Tavily README 仍要求用户“Enable the plugin in global settings”，但 Ora 当前安装模型和 ADR-0001 都把已安装且配置完整的 MCP 视为全局自动参与；实现交付时应同步删改这一步，不能为了迁就 README 再增加一个 enable/disable 状态。
- 研究文档把 OpenCode Adapter 排除在范围外，但真实功能闭环必须修改或升级该 Adapter。
- 部分文档写“恢复后的 Session 已 Ready”，而当前 Runtime 会先把持久化 Session 标为 Stopped，再按需加载。
- 旧 ADR-0010 曾把“配置完整但还没有 Agent Target”定义为 `Ready`，并说等受支持 Agent 第一次打开时再应用；这是 Q90.A 的 lazy 时机，与第 56 问要求沿用 Skills 时机和 Q90.B eager 物化冲突。在 Q88.A 运行时能力发现下，Agent 尚未注册时也无法证明存在兼容消费者，`Ready` 会把“Settings complete”误报成“对话已经可用”。P1 确认后，ADR-0010 已改为 `WaitingForAgent`。
- `CONTEXT.md` 混入了大量实现与决策细节，不再只是领域词汇表；应在方案确定后把决策移回 ADR/Spec。
- 多个可逆实现细节被拆成 Accepted ADR，使尚未验证的实现选择看起来像不可变约束。

P1 确认后，这些漂移已被统一：P3 ADR 标记为 superseded，仍成立的 ADR 按 P1 缩窄，active MCP/Effect spec 与 `CONTEXT.md` 同步更新，研究长文则保留为带醒目历史标记的证据快照。

## 范围选项与决策轨迹

### Q85：首期架构深度

- **A：纵向运行时 MVP**。不建立 MCP Effect/AgentTarget 平台；只完成全局 Ready 集合解析、Agent 启动配置代际、OpenCode 注入和 Tavily 实调。与 Q86.A 组合时范围最小，但会改变原始“物理工作目录文件”验收；与 Q86.B 组合时需要在 Session/启动路径直接写文件，会偏离当前 Skills 的 worker-only 应用方式。
- **B：文件型中间方案（保留原始要求时建议）**。进入现有 Effect Worker 的协调边界，但为 MCP 建立独立、简化的状态；不统一 Skill generation，也不引入派生 AgentTarget 平台。复用 Skills 的声明式、幂等和 wait/restart 原则，而不是复制其目录原子提交实现。
- **C：完整平台重构**。保留当前 ADR 中的统一 generation、AgentTarget、批量重启、复合恢复与生命周期设计，接受多阶段交付范围。

### Q86：首期是否必须产生物理 Workspace 配置文件

- **A：不要求物理文件（改变原始验收）**。验收标准改为“OpenCode 的有效配置中存在 Ora 注入的 MCP，真实调用成功”；使用 `OPENCODE_CONFIG_CONTENT` 和子进程环境变量。只有用户明确接受改变原始要求时才能选择。
- **B：必须写入 Workspace 文件（符合原始验收）**。保留文件合并、用户内容保护和必要的所有权/恢复问题，但仍可删除与首期无关的统一 Skill generation、stdio、完整 AgentTarget、聚合 UI 等范围。

## 逐 ADR 范围处置矩阵

下表不是 ADR 状态变更，而是**只有在用户明确将原始验收改成 `Q85.A + Q86.A` 时**建议采取的后续处置。它区分应保留的领域约束和仅由文件物化方案派生的实现机制；不能用这张表证明物理文件要求已经自然消失。

| ADR                                                | 建议处置                              | 理由                                                                                                                                                                                                         |
| -------------------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| ADR-0001 全局自动启用 Ready MCP                    | **保留核心、删减生命周期扩展**        | 全局启用与首期产品选择一致；精确版本长期保留、Target 独立应用和旧包退役不属于首次调用闭环。                                                                                                                  |
| ADR-0002 Agent Plugin 物化配置                     | **保留并澄清**                        | Host 仍输出 Agent-independent `ResolvedMcp`，OpenCode Adapter 仍负责渲染；“物化”应允许运行时有效配置，不等同于写 Workspace 文件。                                                                            |
| ADR-0003 Workspace Session readiness               | **由 Agent 启动配置门槛取代**         | 需要防止 Session 使用旧配置，但首期门槛应是共享 Agent 的配置 revision，而不是 Workspace × AgentTarget 的 Skill/MCP combined generation。                                                                     |
| ADR-0004 Contract v2                               | **由可选运行时能力取代**              | 保留完整集合和幂等配置，但通过已有注册 method 集合检测 `agent/configureMcp`；不需要静态/运行时双重 v2 声明、stdio、Workspace 参数、Plan/Apply/Observe 或 artifact fingerprint。Host-first 发布顺序可以保留。 |
| ADR-0005 SecretRef 注入                            | **维持 superseded**                   | 首期已经明确使用普通 String Setting。                                                                                                                                                                        |
| ADR-0006 ledger + sidecar                          | **A/A 下后置；Q86.B 才需要**          | 无文件运行时 overlay 不需要声明用户文件所有权，也没有两文件提交问题。                                                                                                                                        |
| ADR-0007 combined generation + composite operation | **A/A 下后置**                        | Settings 自身已有持久 revision；连接 generation 绑定该 revision即可。崩溃会结束旧进程，下一次启动重新解析当前配置，不需要持久化复合操作。                                                                    |
| ADR-0008 String 经环境变量绑定                     | **保留并缩小**                        | 这是不向 Workspace 写明文的必要边界；首期只需覆盖 Tavily HTTP Authorization binding，通用 binding position 规则可在不扩大交付面的前提下保留为纯函数。                                                        |
| ADR-0009 异步 retirement                           | **后置**                              | 首次安装、配置和调用不依赖完整卸载状态机。首期仍应保证删除配置后新连接不再携带 MCP，但不必先实现跨 Workspace ownership-safe cleanup。                                                                        |
| ADR-0010 聚合收敛 UI                               | **缩小**                              | 复用现有 Settings 的 `Needs configuration` / `Ready` 即可；Workspace × AgentTarget 的 Applying/Failed 聚合依赖被后置的 Target 模型。                                                                         |
| ADR-0011 Effect restart 后 Session 失效            | **由 supervisor generation 机制取代** | 配置变化直接重建整个 Agent 连接；现有 connection generation 和 route 失效机制比 plugin 内部 `effect/restart` 再增加 replacement epoch 更贴合运行时。                                                         |
| ADR-0012 派生 AgentTarget                          | **后置**                              | 首期 Ready MCP 集合和 Agent 连接都是全局的，不存在需要投影的 Workspace 差异。                                                                                                                                |
| ADR-0013 纯 resolver 与 Host 安全边界              | **保留 resolver，删除文件分支**       | 纯解析器是深模块边界；Surface、路径、Git、sidecar 和 CAS 校验只属于 Q86.B。                                                                                                                                  |
| ADR-0014 migration 与测试门槛                      | **保留测试原则，重写测试矩阵**        | A/A 不需要新 Effect schema migration。应保留脱敏、fake Agent 集成测试和带真实密钥的发布 smoke，但删除 derived target、文件 crash recovery 等非首期测试义务。                                                 |

按这个矩阵，14 份 ADR 中没有必要“全部推翻”：2 份核心保留，3 份保留但缩小，1 份继续 superseded，8 份被取代或后置。需要避免的是继续把后置 ADR 当成首期验收的依赖图。

### 保留原始文件要求时的 ADR 处置

前表只适用于改变验收后的 A/A 替代方案。若选择 `Q85.B + Q86.B` 并保留 41.A，逐 ADR 判断应改为：

| ADR      | 文件型首期处置                            | 首期边界                                                                                                                                                                               |
| -------- | ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ADR-0001 | **保留自动启用**                          | 保留全局 Ready policy；旧版本长期 retention 可后置。                                                                                                                                   |
| ADR-0002 | **保留**                                  | Host 解析 Agent-independent MCP，Agent Adapter 拥有 OpenCode 文件格式。                                                                                                                |
| ADR-0003 | **拆分**                                  | 保留 worker-only apply、wait/restart 和共享进程的一次批量 activation；全 Session 入口 gate、顺带修复 Skill race 后置到 Q89.B。                                                         |
| ADR-0004 | **由运行时 surface + 受限 renderer 取代** | 使用注册 methods 检测 MCP 能力；OpenCode 插件返回一个 Host-validated Ora-owned 文件计划并持有 activation env，不做 Manifest/Runtime 双 v2 profile、任意 artifact action 或首期 stdio。 |
| ADR-0005 | **维持 superseded**                       | 首期普通 String Setting。                                                                                                                                                              |
| ADR-0006 | **保留核心**                              | 41.A 要求 ledger + sidecar、fail-closed ownership、CAS 和本地路径安全；远程、repair UI 可后置。JSON/JSONC 支持强度需要作为兼容范围显式确认。                                           |
| ADR-0007 | **拆分**                                  | 保留 per-MCP-surface Prepared operation、Settings revision 和 crash recovery；删除 Skill/MCP composite transaction、Target aggregate minimum generation。                              |
| ADR-0008 | **保留**                                  | 文件中只写 env reference，普通 String 明文只进入受信 Agent 进程内存与子进程环境。                                                                                                      |
| ADR-0009 | **后置完整 retirement**                   | 首期至少保证新 generation 不再引用删除项；跨 Workspace 阻塞卸载与旧包 retention 可作为下一里程碑。                                                                                     |
| ADR-0010 | **缩小**                                  | 显示 MCP `Needs configuration / Applying / Ready / Failed`；不展示 Workspace × AgentTarget 聚合拓扑。                                                                                  |
| ADR-0011 | **缩小为 warm invalidation**              | 复用 live actor detach，并显式冷却该 Agent 的 warm bindings；不新增持久 epoch 或完整 Target replacement 模型。                                                                         |
| ADR-0012 | **删除 AgentTarget projection**           | 一个 `agent_mcp_v1` surface 已经是 Workspace × Agent 的持久调度单元；worker 只在本轮内按 Agent consumer 临时分组以合并 restart。                                                       |
| ADR-0013 | **大部分保留**                            | 纯 resolver、Host 路径/Git 安全和 Plan CAS 都由物理文件要求支撑；删除与 deferred transport/remote 有关的分支。                                                                         |
| ADR-0014 | **缩小测试矩阵**                          | 保留 migration purity、脱敏、installer/resolver/ownership/recovery/fake Agent/真实 Tavily 验证；derived Target 和完整生命周期矩阵后置。                                                |

这个文件型矩阵表明：当前设计并非只剩少量内容。41.A 与物理文件使 ADR-0006、0007、0013 的核心合理；共享进程与 eager 全 Workspace 使 ADR-0003 的批量 activation 部分合理。真正应拿掉的是把这些局部需要升级成统一 AgentTarget 平台、全入口 admission、不相关 transport 和完整生命周期。

## 第 41 问的精确答案：sidecar 与现有 Effect 表

### 现有表能否复用

`schema_v0005` 的下列表在结构上是通用的：

- `effect_sources` / `effect_source_revisions`；
- `effect_surfaces` / `effect_surface_consumers`；
- `effect_managed_items`；
- `effect_surface_status` / `effect_consumer_status`；
- `effect_reconcile_requests`；
- `effect_operations` / `effect_operation_artifacts`；
- `effect_conditions` / `effect_audit_events`。

它们可以保存 `effect_kind = 'mcp'`、`format_kind = 'agent_mcp_v1'`、MCP entry 的 `target_json`，因此首期没有证据要求新建 `effect_agent_targets` 或另一套通用 operation 表。

但当前 Rust 实现并不通用：

- `WorkspaceEffectSpec` 只有 `skills`；
- Repository 方法是 `load_managed_skills`、`save_managed_skill`；
- ledger 映射硬编码 `effect_kind = 'skill'`、`target_name` 和 `SkillName`；
- `EffectOperation` payload 内含 `SkillName`、`ManagedSkill`、`DesiredSkillState` 以及一个目录的 staging/backup；
- Planner、scanner 和 filesystem adapter 都按 Skill 目录与 `.ora-managed.json` 工作；
- source propagation 会推进 Workspace generation，并 enqueue 该 Workspace 的所有 surface。

所以准确结论是：**可以复用数据库基础设施，但不能零成本复用当前 Effect 领域实现。** 若把所有上述 Rust 类型一次性泛化成任意 Effect payload，就是平台化重构；若为 MCP 增加少量带类型的分支，则必须避免复制一套平行 worker 和状态机。

还有一个会直接影响改动面的 schema/代码错位：`workspace_effect_desired_items` 在表上允许任意 `effect_kind`，但 `load_workspace_effect` 查询没有过滤 kind，并把每一行都按 `SkillName/DesiredSkillState` 反序列化；直接插入 MCP desired row 会把整个 Workspace Effect 读成损坏状态。新 Workspace 的 v0005 trigger 也只 seed `effect_kind = 'skill'`，而 `enqueue_workspace_surfaces` 会在任一 source 变化时唤醒该 Workspace 的全部 active surfaces。

文件型首期若复用这些表，应接受一个明确而窄的改造：让 Workspace desired snapshot 按 `effect_kind` 分区成 typed Skill/MCP sets，各 surface 只读取自己的集合，但继续共用 Workspace generation 作为粗 change epoch。这样会产生无害的跨 kind 快速观察，却不用新增一套 generation 表。它不等于 Skill/MCP composite Ready；只有 surface/consumer status 才决定各自 Ready。若为了消除这些额外 wakeup 再设计 per-kind/per-surface generation schema，反而是首期没有证据支持的新平台范围。

### sidecar 为什么不能只“直接放数据库”

数据库 ledger 和 Workspace sidecar 证明的是两个不同事实：

- ledger 证明 Ora 的这个安装/profile 曾经管理哪个 identity、source 和 fingerprint；
- sidecar 证明当前 Workspace 文件旁边仍然保留了相同 ownership identity。

只保存在数据库里时，用户删除 Ora 条目后重新创建一个内容碰巧相同的同名条目，或者复制/恢复 Workspace 文件后，旧 ledger 仍可能把新的用户条目误认为自己拥有。逐 entry fingerprint 能降低风险，但不能表达“这个 Workspace 中的当前条目仍携带 Ora 的所有权证据”。

因此，如果坚持与 Skill 相同的**双重所有权保证**，sidecar 必须是目标环境中的独立证据，不能仅换成数据库列。之所以是独立文件，是因为 OpenCode 的 MCP map 没有 Ora ownership 字段，向原生 entry 塞入未知字段可能违反其 schema。

不过，“双重所有权保证”与“首期能安全使用 Tavily”仍是可以单独决策的强度：

- **ledger + sidecar**：最接近 Skill 的 fail-closed 所有权模型，但 config、sidecar、SQLite 跨三个提交边界，必须有 Prepared intent 和 Observe/recovery；这时 journal 不是凭空过度设计，而是 41.A 的直接成本。
- **仅 ledger + entry fingerprint**：可省 sidecar 和复合文件恢复；任何 fingerprint 不匹配都拒绝更新/删除。它更小，但在“用户删除后重建完全相同条目”等边缘情况下无法维持 Skill 等级的双重证明，必须明确记录为首期安全模型降级。
- **仅 sidecar**：Workspace 自包含，但数据库无法可靠协调 Effect status、卸载引用和操作恢复，也不符合当前 Skill ledger 原则，不建议。
- **名称或前缀即所有权**：可能覆盖用户条目，不满足向后兼容要求，不应采用。

所以用户此前选择的 41.A 可以说“与 Skill 的设计原则一致”，但不能说“实现成本与 Skill 一样”。Skill marker 与内容位于同一目录，可用一次目录 rename 提交；MCP config、sidecar 和 SQLite 无法获得同样的物理原子性。

## 保留物理文件时的最小 Effect profile

`Q85.B + Q86.B` 不需要退回当前完整 AgentTarget 设计。一个更窄且与原始要求一致的 profile 是：

1. 全局 Ready MCP 集合仍由已安装 descriptor + 当前 Settings revision 纯解析得到。
2. 每个本地 Workspace × MCP-capable Agent 注册一个 `agent_mcp_v1` Effect surface；该 surface 本身就是调度与状态单位，不再派生第二层 AgentTarget aggregate。
3. Worker 对这个 surface 执行完整集合、幂等 reconcile；OpenCode Adapter 只负责其原生文件格式，Host 继续负责路径、持久状态和调度。
4. 可以复用 `workspace_effects.generation` 作为粗粒度 change epoch，但每个 surface 独立推进 observed/applied/consumer-ready；不承诺 Skill 与 MCP 跨 surface 物理原子，也不计算“所有成员最小 generation”的 Target Ready。
5. MCP 变化导致无关 Skill surface 被 enqueue 时，它只做无 mutation 的快速观察并推进状态；不能因为共享 generation 就声明存在一个 Composite Effect transaction。
6. 首期只支持 Tavily 所需的 HTTP + environment reference；stdio、Workspace context 参数和 sandbox attachment 后置。
7. 如果保留 41.A，则复用现有 operation/managed-item 表，但为 MCP 定义显式、带版本的 payload；不为 AgentTarget 新建表。
8. Settings 保存触发异步 reconcile；用户或测试在 MCP surface 显示 Ready 后开始新会话，真实 Tavily 调用作为闭环证明。

这里的“按 Agent 临时分组”不是当前 worker 已有的一行优化。`EffectWorker::run_pass` 当前最多 claim 16 个请求，然后逐个调用 `reconcile_claimed`；每个 `Reconciler::reconcile_surface` 都自行执行 `quiesce → filesystem operations → resume`，而 `PluginSurfaceCoordinator::resume` 每次都会调用 plugin `effect/restart`。要把 MCP surfaces 合并 activation，必须把文件 Applied 与 consumer Ready 两个阶段在 worker 层拆开，并在等待期间续租组内 claims。这是一个中等规模但边界清楚的 worker 改造，不能计为“免费复用”。

OpenCode Agent `0.3.0` 的 `SkillEffectCoordinator` 也不能充当可靠的跨 Workspace batch：它只有一个进程级 `appliedGeneration: number`，`restart` 在数值相等时跳过 respawn。不同 Workspace 的 generation 是各自演进的计数器；相同数字不证明同一批配置，不同数字也不证明必须再次 restart。MCP 首期应使用 Agent 配置集合 digest 或明确的 transient activation batch identity，不能继续把裸 Workspace generation 当成共享进程配置身份。

最小承诺应表述为：**同一 worker claim batch 中，同一 Agent 对成功 Applied 的 MCP surfaces 最多 activation 一次**。当前 batch 上限是 16，所以 Workspace 数量更大、后续重试或并发新 revision 仍可能产生后续 activation。若产品要求“一次全局 Settings 变化跨任意数量 Workspace 严格只 restart 一次”，就必须持久化变更 cohort、等待所有成员终态并定义部分失败/超时；这正是被建议后置的 AgentTarget/activation-batch 平台，不能继续称为最小闭环。

这一路径仍需要修改 Effect 领域层和 Repository，工作量高于无文件替代方案；但它保留了原始文件要求，也避免了当前 ADR 中最昂贵的 derived AgentTarget、跨 Skill/MCP composite readiness、批量多 Target activation 和全生命周期 retirement。

这里还存在一个必须公开的保证强度差异：如果产品要求“任何 interactive、warm、恢复或 workflow prompt 在 surface Ready 前都绝不进入 Agent”，就仍然需要 ADR-0003 类型的中央 admission barrier；如果首期验收只是“Settings 明确显示 Ready 后，新建对话可以调用 Tavily”，全路径 admission hardening 可以后置。前者是更强产品不变量，不能被包装成写配置文件的自然附属品。

OpenCode Agent 0.3.0 的 [`SkillEffectCoordinator`](https://github.com/ora-space/opencode-agent/blob/v0.3.0/src/handlers/effects.ts) 已经追踪进行中的 `session/prompt`：当全部 turn 结束后，它持有 barrier，把后来到达的 prompt 暂存，等 `effect/restart` 完成后再按顺序重放。因此 `Q85.B + Q86.B` 可以复用 Agent 插件已有的 wait/restart 协调，不必仅为了“等待活跃 turn”就在 Host 建立完整活动计数平台。

但当前 Host 的 `detach_sessions_for_replaced_plugin` 只通知持久化 live actor，没有主动冷却 warm pool；plugin 内部 restart 也不会推进 Host supervisor 的 connection generation。这是一个真实但较窄的缺口：首期可以增加“按 Agent 使 warm provider binding 变 cold”的显式操作，并继续使用现有 actor detach。只有要求所有 session lifecycle 入口在整个窗口内统一线性化时，才需要升级成 ADR-0003/0011 描述的完整 admission/replacement 机制。

## 可以缩小 JSONC 合并，但会限制首期兼容面

OpenCode 1.18.25 的配置加载器会合并多层来源。项目文件之后，它还会在 `.opencode` 目录中依次加载 `opencode.json` 和 `opencode.jsonc`；后加载的值覆盖相同 map key。源码同时已经使用 `jsonc-parser` 做局部更新：

- [`config.ts`](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/opencode/src/config/config.ts)；
- [`paths.ts`](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/opencode/src/config/paths.ts)。

这提供了一个比 ADR-0006 更窄的文件型首期：

1. Ora 固定管理 `.opencode/opencode.jsonc`，仅在该文件不存在时创建。
2. 文件包含 `$schema`、完整 Ora MCP map 和不含敏感值的 ownership marker；API Key 仍然通过 env reference。
3. Ora 拥有整个文件，可以通过临时文件 + rename 更新，不需要对用户 JSONC 做语法树合并。
4. 如果目标文件已经存在，Ora 报 `PreservedConfigConflict`，不修改它；支持共同编辑现有 JSON/JSONC 作为下一里程碑。
5. 如果 marker 与 DB ledger 放在同一个 JSONC 文件的固定头部注释中，内容与 marker 可以一次文件 rename 提交，物理形态重新接近 Skill 的“内容 + marker 同一原子单元”。这会取代独立 sidecar，但仍保留双重所有权证明和数据库 Prepared operation。

这个方案满足“物理写入 Agent 工作目录配置文件”和干净 Workspace 的 Tavily 闭环，也保护既有文件不被破坏；代价是首期在已经使用 `.opencode/opencode.jsonc` 的 Workspace 中明确不可用。若首期必须对已有 OpenCode 项目普遍可用，就必须保留 ADR-0006 的 JSON/JSONC 合并与 per-entry ownership，不能把它们简单称为过度设计。

对 OpenCode 1.18.25 源码的进一步验证确认了这个文件形态的三个前提：

- [`ConfigParse.jsonc`](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/opencode/src/config/parse.ts) 使用 `jsonc-parser`，接受注释与 trailing comma；
- [`ConfigMCPV1.Remote`](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/core/src/v1/config/mcp.ts) 接受 `type = remote`、`url`、`headers`、`enabled` 和可选 timeout/oauth；
- [`ConfigVariable.substitute`](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/opencode/src/config/variable.ts) 在 JSONC 解析前替换 `{env:VAR}`，因此 Header 可以只持久化环境引用。

一个不含 Secret 的示意文件可以是：

```jsonc
// ora-managed-mcp.v1 eyJ3b3Jrc3BhY2VJZCI6Ii4uLiIsInN1cmZhY2VLZXkiOiIuLi4ifQ
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "ora__ora-space__tavily-search": {
      "type": "remote",
      "url": "https://mcp.tavily.com/mcp",
      "enabled": true,
      "headers": {
        "Authorization": "Bearer {env:ORA_MCP_TAVILY_API_KEY}",
      },
    },
  },
}
```

Marker payload 应包含 schema version、Workspace/surface identity、完整 desired digest，以及每个 managed key 对应的 managed identity；示例只缩写其形态。它不能包含 Setting 值。`$schema` 必须由 Ora 一开始写入，避免 OpenCode 因补 schema 而主动改写这个 Ora-owned 文件。

因为整个文件属于 Ora，Materialization 可以按一个完整 MCP 集合执行：一次 Prepared operation 记录 previous/planned file fingerprint，一次临时文件 rename 同时提交 marker 和内容，最后一个 SQLite transaction 批量更新该文件内各 MCP source 的 ledger rows 并 Finalize。现有 `effect_operations` 的 staging/backup 结构可以复用；Rust payload 和 `finalize_operation` 需要增加 MCP 文件 variant 与批量 ledger transition，但不再需要 sidecar artifact、双文件 CAS 或 JSONC syntax-tree edit。

仓库内已经存在支撑这一收敛方案的底层能力，但不能把它误解成现成的 MCP 事务：

- `ora_utils::atomic::write_with_prepare` 在目标目录创建 `NamedTempFile`，写入、flush、`sync_all`，再 `persist` 到目标；读者不会看到半个 JSONC 文件。
- `ora-plugin-config` 的 `ConfigurationFileSystem::atomic_write` 已复用该能力，并在 Unix/Windows 上为临时文件设置受限权限。这证明首期不需要再实现一套通用 atomic-file helper。
- 当前 Effect 的 Prepared / Applied / Finalized 恢复和 previous/planned fingerprint 是目录型 Skill 操作，`FilesystemSurfaceAdapter`、`EffectOperation` 和 `finalize_operation` 仍然是 Skill-specific，不能直接拿来写 MCP 文件。

因此更准确的实现边界是：复用 `ora-utils` 的单文件原子替换和 Effect 的 durable-operation **协议**，为 `agent_mcp_v1` 增加文件 payload；不复用 Skill 的目录 staging adapter，也不把所有 Effect payload 一次性泛型化。创建或更新时，Prepared row 记录 absent/previous 与 planned digest，原子替换后若进程崩溃，恢复只需在 absent、previous、planned、unknown 四种观察结果之间做穷举决策；删除最后一个 Ora MCP 时才需要把整个 Ora-owned 文件 rename 到 operation backup 后再 Finalize。多 MCP entry 共享一个文件，所以 ledger 变更必须在同一个数据库 transaction 中批量提交。

`atomic::write` 只同步了临时文件本身，没有额外同步父目录；它足以处理普通进程崩溃下的半写可见性，不能单独宣称数据库与文件系统在突然断电时具有跨介质事务保证。如果首期验收只是应用进程终止并重启，现有 Observe/recovery 模型足够；把断电级持久性也纳入验收会新增明确范围，不应从“crash recovery”四个字自动推导出来。

Git local exclude 仍是另一个文件，但不需要和 MCP JSONC 做复合原子提交：先幂等写好 exclude 再发布配置，崩溃最多留下无害的 stale exclude；若 exclude 失败，配置文件尚未出现。删除最后一个 MCP 后可以保留该条目或另行清理，它不影响 ownership 证明。这样 ADR-0006 中 Host 负责 Git policy 的边界仍可保留，而不会重新引入 config + sidecar + SQLite 的三方提交问题。

环境替换是 OpenCode 的既有能力，但不能把普通 Setting 原值直接设为 `{env:...}` 对应的环境值：OpenCode 在 JSONC 解析前做文本替换，引号、反斜杠或换行会破坏所在 JSON string，解析错误还可能包含替换后的输入。首期不必猜测 Tavily 未正式承诺的完整 key 字符集，也不必引入 Secret 类型；OpenCode Adapter 可以先把实际值序列化成 JSON string，再去掉外围引号，把得到的 **JSON string content** 作为子进程环境值。替换后的 JSONC 始终保持语法有效，解析后 Header 得到原始值。resolver 仍必须拒绝最终 Header 中的控制字符，避免 Header injection。

[`ConfigVariable.substitute`](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/opencode/src/config/variable.ts) 还有一个顺序细节：它先执行 env replacement，再在替换结果中扫描 `{file:...}`。所以标准 JSON escaping 后还必须把 `{`、`}` 编码为 `\u007b` / `\u007d`；否则一个含 `{file:...}` 的 Setting 值会被当成 OpenCode 文件引用。JSONC 最终解析时这些 unicode escape 会恢复成原始花括号，不改变发送给 MCP 的 Header。该编码应是 OpenCode Adapter 的纯函数和测试矩阵，不应污染 Agent-independent resolver。

本地用含引号、换行和 `{file:...}` 的合成 key 复现了 OpenCode 的两阶段替换：上述编码后 file-token 命中数为零，JSON parse 后 Header 与原始值逐字一致，替换后的 JSON 文本也不包含原始 key 连续字节。这个测试证明编码路径可行；它不替代真实 Tavily smoke，也不证明 OpenCode 的其他诊断永远不会输出 Header，因此 stderr redaction 仍然需要。

日志边界也不能由“第一阶段允许明文存储”推导为“允许明文写日志”。当前 OpenCode Agent 会把 CLI stderr 原样写到自己的 stderr，Ora Plugin Runtime 又把插件 stderr 完整记录为结构化日志。Agent Adapter 已经持有 pending environment 的实际值，因此在转发 OpenCode stderr 前至少要精确替换已知原值及其 JSON-escaped 表示，并保证 Host 对 renderer/configure IPC 的错误映射不序列化请求参数。这样首期仍是普通 String + 进程内脱敏，不扩大成 SecretRef、密钥库或通用 DLP 系统。

### 当前设计遗漏的跨配置层冲突

无论采用创建专属文件还是合并现有文件，OpenCode 的有效配置都来自多个层。一个 `.opencode/opencode.jsonc` 中的 Ora entry 可能覆盖以下位置的同名用户 MCP：

- OpenCode 全局配置；
- `OPENCODE_CONFIG` 指向的自定义文件；
- Workspace 祖先目录的 `opencode.json` / `opencode.jsonc`；
- 同一 `.opencode` 目录的另一个配置文件；
- 外部 `OPENCODE_CONFIG_CONTENT`；
- 系统 managed config。

ADR-0006 当前只描述目标 Workspace 文件中的 Preserved entry 和 sidecar。若 Adapter 没有观察其他配置层，它无法兑现“任一同名用户 entry 都必须冲突”，而是会在不修改原文件字节的情况下运行时 shadow 用户 MCP。这是设计完整性缺口，不是范围大小问题。

首期必须在下列语义中明确选择一种：

- 使用 Ora 保留、可读且碰撞抵抗的 Agent MCP key，并把无法观察到的外层同 key shadow 明确视为极低概率但可逆的已知限制；
- 由 OpenCode Adapter 完整枚举 1.18.25 的所有可观察配置层，发现同 key 就冲突；这提高兼容保证，但把 OpenCode 配置解析与环境发现纳入首期；
- 改变产品语义，允许 Ora 层有意 shadow 同名用户 entry，并明确移除 ADR-0006 的“冲突不接管”承诺。该行为虽然可逆，仍会改变用户对话时的有效 MCP，不应默认采用。

sidecar 或 inline marker 只能回答“我是否拥有准备更新的 entry/file”，不能替代上述有效配置冲突决策。

## 与 active specs 的关系

仓库当前 `specs/active/effect/*` 和 `specs/active/plugin/5-mcp.md` 已经描述了一个比实际代码更完整的长期模型，包括：

- Workspace-scoped MCP selection 与 generation；
- `Workspace × AgentPlugin` AgentTarget；
- Skill/MCP 原子 reconcile；
- SecretRef；
- Agent 原生文件 materialization、ownership 和 Session close/resume；
- stdio MCP 的 Workspace cwd 与沙盒资源。

这些文件位于被 Git 忽略的 `specs/` review source 中，当前代码尚未实现上述 MCP 模型。它们证明平台化方向并非完全由本轮 ADR 临时创造，但不能反过来证明所有长期设计都必须进入 OpenCode + Tavily 首期。

因此，`Q85.A + Q86.A` 同时改变了原始物理文件验收，也不能被描述成“完全遵循现有 active specs”。如果用户主动选择它，应当被明确记录为一个受限的首期 profile：

1. 只支持全局、Workspace-independent、HTTP MCP；
2. 使用共享 Agent 的 launch-time effective config，不执行 Workspace 文件 materialization；
3. 保持 `ResolvedMcp → Agent adapter` 的长期边界；
4. 明确不实现当前 Spec 中的 Workspace selection、SecretRef、stdio、AgentTarget 和文件 ownership；
5. 后续出现 Workspace-dependent MCP 时，再进入既有长期 Effect 模型，而不是让首期临时路径悄悄长成第二个并行平台。

如果不愿意改变原始验收，应优先讨论 `Q85.B + Q86.B` 的文件型中间方案。若还要求完整遵守当前 active specs，才需要选择 Q85.C 的完整平台化方案，或者先正式修订 active specs；不能一边声称完整遵守它们，一边只实现全局环境注入。

## A/A 替代方案仍然不可省略的并发边界

无文件方案减少了持久化状态机，但不等于“Settings 保存后随便重启一下”。至少要满足以下线性化约束：

1. 一次 Agent connection generation 绑定一个不可变的 MCP configuration revision/digest。
2. Host 在 Agent Plugin 注册完成后、调用 `agent/start` 之前发送完整 MCP 集合；OpenCode 子进程从第一次启动起就携带对应配置与环境。
3. Settings 保存或 MCP 安装状态变化后，旧 connection generation 不能继续无限期接受使用旧 MCP 集合的新工作。
4. 已经运行的 turn 必须采用首期明确的策略：立即终止并重建，或者先进入全局 admission/quiesce 再等待结束；不能在同一 OpenCode 进程中偷偷切换配置。
5. 新 generation 完成 Agent 启动和 ACP initialize 后才能重新开放 admission。
6. 进程在“保存成功、重启通知前”崩溃不需要 durable reconcile request：应用重启后必须从当前 `store.json` revision 重新解析，再启动 Agent；旧进程已经不存在。

这个门槛可以复用现有 supervisor 的 connection generation、route failure 和 Session stopped 语义。Warm pool 已经按 connection generation 识别并冷却旧 provider session，因此走完整 supervisor replacement 时，不需要 ADR-0011 额外定义的 plugin-transport 内 `Agent Replacement Epoch`。仍需新增“配置变化触发受控 replacement”的入口；它是最小闭环内真实存在的并发工作，不应被范围收缩错误地删掉。

“等待活跃 turn 自然结束”不是免费增强。要同时保证等待期间没有交互 prompt、warm Session 或 workflow 节点重新进入，Host 必须建立 Agent-global admission gate 和可靠的活动计数；这会重新引入 ADR-0003 的一部分复杂度。若首期选择立即 replacement，可以复用现有连接失败路径并明确提示用户，但设置变化会中断正在进行的 turn。这是产品取舍，不应藏在实现细节中。

## A/A 替代方案的实际改动面

现有代码已经提供了五个可复用接缝：

- Plugin Configuration 的 `store.json` 已经有单调 revision 和 compare-and-save；
- Plugin Runtime 注册结果已经暴露完整 method 集合，可以进行运行时能力检测；
- Agent attach 当前先取得注册结果，再调用 `agent/start`，中间存在配置完整集合的自然注入点；
- Plugin SDK 与 Host child-process 已经支持环境变量 overrides；
- Agent supervisor、route registry 和 warm pool 已经围绕 connection generation 处理连接更替。

因此 A/A 仍然是跨仓库功能，但边界是可枚举的：

| 模块                         | 首期必要变化                                                                  | 不需要进入的模块                             |
| ---------------------------- | ----------------------------------------------------------------------------- | -------------------------------------------- |
| `ora-plugin-manager`         | Marketplace archive 的内外层 Manifest 身份/版本/kind 一致性校验               | Effect schema、Target ledger                 |
| `ora-plugin-config`          | 从已编译 MCP + 当前 values/revision 纯解析 `ResolvedMcp`                      | Agent-native JSON/JSONC 渲染                 |
| Backend Plugin/Configuration | 枚举全局 Ready MCP；安装、卸载、保存、重置后触发 Agent 配置变化               | Workspace selection 与 per-target 状态       |
| Backend Agent Runtime        | 在 `agent/start` 前发送完整集合；connection 绑定配置 digest；受控 replacement | Workspace Effect Worker、复合 operation      |
| Plugin SDK                   | Agent definition 的可选 MCP configure handler 与 DTO                          | Plan/Apply/Observe、artifact protocol        |
| OpenCode Agent 0.4           | 生成 `OPENCODE_CONFIG_CONTENT` 与 env overrides 后启动 OpenCode               | Workspace 文件编辑、sidecar、Git             |
| 验证                         | installer/resolver 单测、fake Agent 集成测试、真实 Tavily release smoke       | crash-recovery 文件矩阵、Target 聚合 UI 测试 |

这不是“几行配置注入”，但也不是当前 ADR 描述的数据库 + Effect + 文件系统平台重构。主要新增复杂度集中在一条 Agent launch/replacement 路径，能够由一个配置 digest 贯穿测试。

## 后续范围决策轨迹

### Q87：MCP 配置变化时如何处理活跃 turn

- **A：复用现有 Effect wait/restart（保留文件方案时建议）**。OpenCode Agent 等待已开始的 prompt 完成、持有后续 prompt、重启 CLI 并重放；Host 补充 live actor detach 之外的 warm binding 冷却。它保持与 Skills 相同的协调时机，但不宣称覆盖所有 session lifecycle admission。
- **B：Host 全局 quiesce**。在 interactive、warm、恢复和 workflow 的所有入口建立中央 gate，等待所有 turn Idle 后 replacement。保证最强，但需要保留 ADR-0003 的主要 admission/activity 设计，应与 Q89.B 一起选择。
- **C：立即替换完整 Agent connection**。正在运行的 turn 以明确的 `AgentReconfigured` 类错误停止，旧 Session 依照 connection generation 机制失效；无文件方案最容易采用，物理文件方案也可采用，但会偏离现有 Skills 的用户体验。
- **D：只在 Ora 下次启动时应用**。实现最小，但 Settings 保存后的行为具有欺骗性，也不能完成不重启应用的功能闭环，不建议。

### Q88：首期如何声明 Agent 的 MCP 能力

- **A：运行时 surface + 受限 renderer（建议）**。Agent 注册包含 `agent_mcp_v1` surface 和可选 MCP render/configure method 就参与 MCP 物化，不包含则继续按旧 Contract 提供普通对话；插件返回 Host 验证和写入的一个完整文件计划，并在内存中接收 activation 所需 env，不自行任意写 Workspace。OpenCode 0.4 通过已有注册 method 集合暴露能力。
- **B：完整 Contract v2**。在安装 Manifest 和运行时注册中重复声明版本、transport、binding forms，并做交叉校验。它适合未来在 Agent 启动前展示静态兼容矩阵，但不是 OpenCode 单 Adapter 闭环的必要条件。

现有注册协议已经给出 method capability，因此 Q88.A 不是绕开契约，而是复用现有契约协商方式。若以后需要不启动 Agent 就判断兼容性，再增加静态 Manifest profile 会更有证据支持。

但 Q88.A 仍需澄清“谁拥有 OpenCode JSONC”。当前 Host 只接受 `skill_directory.v1`，并由 Host 的 filesystem adapter 写目录；Agent 插件只声明路径并协调 wait/restart。若首期让 Host 直接生成 OpenCode MCP JSONC，代码量最小，却把首个 Agent 的原生 schema 放入 Ora core，实质偏离 ADR-0002。反过来，让 Agent 插件直接任意读写 Workspace，则绕开 Host 的 containment、Git policy、ownership ledger 和 Prepared operation，安全边界更差。

与 Q91.A 配套的中间接口应是一个**受限的 runtime renderer**：OpenCode Agent 注册 `agent_mcp_v1` surface 和可选 MCP 方法；Host 传入完整、规范化且以 environment reference 表示敏感值的 MCP set 与 ownership 元数据，插件只返回固定 surface 下的单个完整文件计划/bytes 和 digest，Host 验证 locator、大小、digest、当前 ownership 与 Git 前置条件后原子落盘。实际 Setting 值只通过受信 IPC 进入插件的 pending environment map，`effect/restart` 用它重启 OpenCode CLI，不出现在返回计划、operation payload、marker 或日志中。进程若在 Apply 与 restart 间退出，下一次 reconcile 从 exact Settings revision 重新调用 renderer，不依赖丢失的插件内存。

这个接口需要扩展 Plugin SDK、runtime registration 和 Host adapter，但不需要 Contract v2 的安装 Manifest 静态 profile、transport matrix、任意多 artifact、插件自行 Apply 或通用 Observe action。若选择 Q91.B 合并用户文件，renderer 已不足以处理读取、局部 edit 与 CAS，完整 Plan/Apply/Observe 的理由才重新成立。

### Q89：首期需要多强的 Session admission 保证

- **A：Ready 后使用（建议用于最小闭环）**。Settings/Effect 明确展示 `Applying` 与 `Ready`；用户和端到端测试只在 Ready 后创建新对话。Ready 前尝试使用可以返回“配置正在应用”，但不为所有 warm、恢复和 workflow 路径建立统一 generation barrier。
- **B：任何路径都绝不使用旧 generation**。在 interactive session/new/load、warm create/reuse、prompt、workflow node 等全部入口建立同一个 AgentTarget admission barrier。这是更强且有价值的产品不变量，但会保留 ADR-0003 的主要运行时改造。

Q89.A 不是允许 Ready 状态撒谎：Surface 只有文件、所有权记录和 Agent restart 都成功后才能 Ready。它缩小的是“Ready 之前所有现有调用路径都必须主动等待”的覆盖面，而不是降低 Ready 本身的真实性。

现有 Plugin Configuration `summary` 只表达声明和值是否完整，首期不应把它重载成运行状态。无需持久 AgentTarget aggregate，也可以通过现有 desired/managed/surface/consumer rows 在读取时派生一个较窄的 MCP application summary：`NeedsConfiguration`、`WaitingForAgent`、`Applying`、`Ready`、`Failed`。在 Q88.A 下，没有任何 MCP-capable Agent runtime declaration 时应是 `WaitingForAgent`，不是 `Ready`；在 Q90.B 下，所有已注册本地 surfaces 的 consumer-ready 状态都追上后才是 `Ready`。这需要新增 read-model/contract/UI 状态，但不需要新的 Workspace MCP 设置页或 `effect_agent_targets` 表。

### Q90：全局自动启用是否意味着立即写入所有既有 Workspace

- **A：按需物化（需要重新打开第 56 问）**。所有 Ready MCP 对每个 Workspace 都自动适用且不可排除，但只在 Workspace 被打开、选择该 Agent 或准备创建首个对话时创建/唤醒 MCP surface；Ready 后才开始对话。它减少批量重启，却增加一个不同于当前 Skills 的触发时机，不能在不改变既有选择的情况下采用。
- **B：所有 Workspace eager 物化、每个 claim batch 合并 activation（保持 Skills 时机时建议）**。Settings/安装变化立即为全部本地 Workspace 收敛，成功 surface 独立前进；worker 在当前最多 16 个 claims 的 batch 内按 Agent consumer 分组并最多 activation 一次，不需要持久化或派生 AgentTarget aggregate。Workspace 超过 batch、后续重试或新 revision 可以产生后续 activation；本选项不承诺一次全局变化严格 only-once restart。
- **C：所有 Workspace eager 物化、每个 surface 各自重启**。最接近直接复用当前 Skill worker，但一个全局 MCP 变化可能连续重启同一个 Agent 多次，对现有用户影响不可控，不建议。
- **D：所有 Workspace 完成后严格只 activation 一次**。持久化全局变更 cohort，等待所有成员成功、失败或超时后再决定共享 Agent activation，并定义新 revision supersede 与部分失败；保证最强，但会重新引入 activation-batch/AgentTarget 聚合平台，不属于最小闭环。

Q90.A 与 Workspace opt-out 是不同维度：前者决定“什么时候应用”，后者决定“是否应用”。按需并不会让某个 Workspace 选择不启用 Tavily。

### Q91：首期是否必须合并用户已经存在的 OpenCode 配置

- **A：只创建 Ora-owned `.opencode/opencode.jsonc`（最小闭环建议，但会重开 41.A）**。目标不存在时创建，存在时 fail closed；ownership marker 可放入同一 JSONC 文件头部，从而以“DB ledger + colocated marker”继续双重证明，同时取消独立 sidecar。首期不修改任何用户原有配置文件。
- **B：支持现有 JSON/JSONC 合并**。保留注释、缩进、换行、字段顺序和无关字段，并对 Ora-managed entry 做 per-entry sidecar/ledger proof。覆盖面更实用，但 ADR-0006/0007 的大部分文件复杂度都成为合理范围。

### Q92：同名 MCP key 的冲突保证覆盖哪些 OpenCode 配置层

- **A：Ora 保留的碰撞抵抗 key + 已知限制（最小闭环建议）**。例如由 canonical Plugin ID 派生可读稳定前缀；扫描 Workspace 可见层并 fail closed，无法观察的全局/managed 外层碰撞记录为可逆限制。
- **B：完整层级冲突检测**。Adapter 按捆绑的 OpenCode 1.18.25 规则枚举全局、自定义、祖先、`.opencode`、content 和 managed 来源，任何同 key 都阻止物化。保证最符合 ADR-0006，但扩大 OpenCode-specific 适配与测试矩阵。
- **C：允许 Ora shadow**。不修改用户源文件，移除 Ora overlay 即恢复，但对话中的有效能力会被替换；只有明确改变“用户 entry 冲突不接管”产品语义后才能选择。

### Q93：配置完整但尚无 MCP-capable Agent 时显示什么

- **A：`WaitingForAgent`（建议）**。Settings 值已经完整，但还没有运行时 Agent capability/surface，不能声称 MCP 已可用于对话；Agent 注册后立即进入 eager `Applying`，全部既有本地 surfaces consumer-ready 后才显示 `Ready`。
- **B：`Ready`**。沿用 ADR-0010，把 Ready 解释成“配置本身已就绪，未来首次打开 Agent 时再应用”。这与 Q90.B eager 和 Ready 后可用的通常含义冲突，只有改回 lazy 或重命名为 `ConfigurationReady` 才不误导。
- **C：不显示 application summary**。只保留现有 Settings completeness，应用失败等到创建 Session 时报告。改动最少，但异步文件冲突和 restart 失败缺少可见反馈，不建议。

## 选择之间的依赖：不要拼出一个隐性平台

Q85–Q93 不是九个互不相关的开关，以下组合会自相矛盾或隐藏范围：

- `85.A + 86.B` 会把 Workspace 写文件塞进 Agent start/Session 路径，既没有文件 Effect 恢复，又违背第 56 问要求沿用 Skills worker 时机；不建议作为折中。
- `87.B + 89.A` 名义上只要求 Ready 后使用，实际上却先建设了全局 quiesce；若确实要 Host 全局 gate，应同时承认 `89.B` 的平台范围。
- `89.B` 要覆盖 interactive、warm、load、resume、workflow 的统一 admission，因此与 `85.C` 的一部分绑定，不能继续称为文件 adapter 局部需求。
- `90.A` 明确重开第 56 问；`90.D` 则需要 durable cohort/aggregate，等价于把被删掉的 activation Target 平台加回来。
- `91.B` 不能使用 Q88.A 的 create-only renderer 原样实现。它至少要让 OpenCode Adapter接收 Host bounded-read 的 observed JSON/JSONC、做保真 merge并返回 previous/planned bytes/digest，Host 再 CAS + 原子写；同时恢复独立 per-entry sidecar。它不逻辑强制安装 Manifest Contract v2，但会恢复大部分 runtime Plan/Observe 与多 artifact 测试。
- `92.C` 与 41.A 的“冲突不接管”所有权承诺矛盾；除非正式改变产品语义，否则不能仅因为实现简单而选择。
- `93.B + 89.A` 会让“Ready 后可用”失去含义；若无 Agent 也叫 Ready，就必须改名为 `ConfigurationReady`，并另设 application 状态。

更容易决策的四个整体 profile 是：

| Profile                             | 选择                                                   | 保留什么                                                | 明确代价                                                               |
| ----------------------------------- | ------------------------------------------------------ | ------------------------------------------------------- | ---------------------------------------------------------------------- |
| **P1 原始验收的窄文件闭环（建议）** | `85.B/86.B/87.A/88.A/89.A/90.B/91.A/92.A/93.A`         | 物理文件、Skills 时机、双重 ownership、Ready 后真实调用 | 仅 clean target file；batch 内合并 activation；没有全入口 gate         |
| **P2 兼容既有 OpenCode 项目**       | P1，但 `91.B`，并扩展 runtime merger；`92.A` 或 `92.B` | 可以安全编辑现有用户 JSON/JSONC                         | JSONC 保真、per-entry sidecar、双文件恢复与 CAS 回归首期               |
| **P3 强保证平台**                   | `85.C/86.B/87.B/88.B/89.B/90.D/91.B/92.B/93.A`         | 全入口无 stale、全局 cohort、广泛配置兼容与静态能力判断 | 当前 ADR 的大部分平台工程均合理，不能再称最小闭环                      |
| **P0 运行时验证原型**               | `85.A/86.A/87.C/88.A/89.A/93.A`；90–92 不适用          | 最快证明 Tavily 可被 OpenCode 调用                      | 明确删除原始物理文件验收，只能作为 P1 前置 prototype，不能冒充交付完成 |

P1 和 P2 的差别不是“多支持一种扩展名”，而是 ownership 原子单元从一个 Ora-owned 文件重新变成用户 config + sidecar + DB 三边协议。P1 和 P3 的差别也不是测试强度，而是是否建立跨任意 Surface/Session 生命周期的持久协调平台。

## 文件型最小闭环的实际工作包与停止线

即使选择推荐的 `B/B`，这仍是一个跨仓库纵向功能，不应被估成“安装两个插件后写一段 JSON”。它的合理工作包是：

| 工作包                        | 主要现有接缝                                                                            | 相对深度 | 首期停止线                                                                                                                                       |
| ----------------------------- | --------------------------------------------------------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Installer pre-commit identity | `ora-plugin-manager::Installer`、backend install tests                                  | 小       | 只补 universal/targeted 包内外 namespace/name/version/kind；不建签名或安装事务平台                                                               |
| MCP pure resolution           | `ora-plugin-config` compiler、`store.json` revision/details                             | 中       | 输出 HTTP `ResolvedMcp`/NeedsConfiguration；保留已有 stdio AST，但不实现 stdio runtime                                                           |
| Ready source synchronization  | startup `sync_installed_skills`、save/reset/install/update 成功边界、Effect source rows | 中       | coalescing source refresh + startup repair；不把文件写与 SQLite 伪装成单事务                                                                     |
| Runtime Agent adapter seam    | Plugin SDK/runtime registration、`plugin_agent::effect`、OpenCode Agent                 | 中       | 一个 `agent_mcp_v1` surface、一个受限单文件 renderer、safe env encoding/pending env/stderr redaction；不做静态 Contract v2 matrix 或 Secret 系统 |
| MCP file Effect profile       | v0005 generic tables、typed Effect payload、`ora-utils::atomic`、Host Git/path checks   | 大       | Ora-owned create-only JSONC、inline marker、批量 ledger finalize；不编辑用户 JSONC、不建 AgentTarget 表                                          |
| Shared activation             | `EffectWorker` claims、wait/restart、live detach/warm pool                              | 大       | 每个 claim batch/Agent 最多一次 activation；不保证跨任意 Workspace 全局 only-once、不建全入口 gate                                               |
| Truthful status + E2E         | Plugin Settings contracts/UI、surface/consumer status、Tavily smoke                     | 中       | `WaitingForAgent/Applying/Ready/Failed` 读取投影和一个真实调用；不建 Workspace MCP 管理页或聚合拓扑 UI                                           |

这七个工作包说明 B/B 的开发面仍然明显，但每个深模块只有一个首期 profile。完整 ADR 方案不是因为“文件型方案碰了很多 crate”才过度，而是它在每个工作包里同时引入了未来变体：任意 Agent/transport/artifact、既有用户配置 merge、跨 Surface Target、严格全局 activation、全 Session admission、retirement 和远程环境。范围控制应靠上述停止线，而不是把必要的跨模块链路删成无法端到端验证的局部实现。

当前 `desktop-mcp` Workspace 中 `.opencode/opencode.json`、`.opencode/opencode.jsonc`、根级 `opencode.json/jsonc` 均不存在，所以 Q91.A 可以直接作为本仓库的真实 E2E fixture。这个事实只证明 clean-workspace 验收可行，不证明现有用户普遍没有同名配置；产品说明必须明确“目标文件已存在时首期不支持并保持不修改”。

## 首期 Done 的证据，而不是平台愿望清单

已确认的 P1 只有在以下证据同时成立时才完成闭环：

1. Marketplace 安装测试证明外层 listing digest 错误、包内 Manifest 缺失或 namespace/name/version/kind 不一致都在最终 rename 前失败，目标安装目录不存在；当前 OpenCode `0.3.0` 和 Tavily `0.1.0` 正常安装。
2. Resolver 单测证明缺失/空白/类型错误 API Key 得到 `NeedsConfiguration`，合法 Tavily HTTP 定义得到稳定 complete-set digest；Effect payload、marker 和 renderer output 不含实际 Key。
3. Adapter 纯函数测试证明 JSONC、ownership header、保留 key、env name 和 digest 可重复；含引号、反斜杠、控制字符与 `{file:...}` 的合成 Setting 不破坏 JSON、不会触发 file substitution，最终非法 HTTP Header 被拒绝。
4. 文件 Effect 故障测试覆盖 absent/previous/planned/unknown observation、Prepared 后退出、Apply 后 Finalize 前退出、用户占用目标、marker/ledger/fingerprint 不匹配，以及最后一个 MCP 的安全删除；任何冲突都不修改用户文件。
5. Worker 集成测试证明 Settings save/reset、MCP install/update 和启动 repair 会收敛现有本地 Workspace；同一 claim batch 的同一 Agent 只 activation 一次，失败 surface 不被误报 Ready，live/warm binding 按首期策略失效。
6. Settings UI 明确区分 `NeedsConfiguration`、`WaitingForAgent`、`Applying`、`Ready`、`Failed`；只有文件、ledger 和 Agent activation 都确认后才是 Ready。
7. 一个显式 opt-in 的真实测试使用环境提供的 `TAVILY_API_KEY`：从 Marketplace 安装 OpenCode MCP-capable release 与现有 Tavily `0.1.0`，保存配置，观察 Ora-owned Workspace JSONC 中只有 env reference，在 Ready 后新建对话，观测一次 Tavily tool-call 及成功结果，并确认日志和测试输出不含 Key。

第 7 项是原始“对话中真实使用”的验收证据，但不自动要求每个 PR 联网、把真实密钥放入普通 CI，或立即建设 release-gate 服务。首期可以提供 hermetic fake-Agent 测试和一个手动/受保护环境运行的 opt-in smoke；是否升级成强制 release gate 是独立发布策略。

以下测试矩阵不属于该 Done：第二种 Agent、stdio 子进程、远程 Workspace、用户 JSONC 合并保真、超过一个 claim batch仍严格 only-once、所有 workflow/load/warm 入口的统一 admission、完整卸载 retention、断电级目录持久性，以及 Workspace MCP opt-out。把其中任何一项加入首期，都应明确重新估算，而不是归入“补几个边界测试”。

## 审计结论

当前设计曾存在明确的范围扩大和设计重心偏移，但需要区分“原始要求的真实成本”和“额外平台投资”。MCP 必须物化为 Agent 工作目录配置文件来自原始验收，不能被归类为设计自行扩大；被 P1 否定的两个推导是：物理文件因此必须完全复制 Skill Effect 的状态形态，以及首期因此必须建立通用多 Surface/多 Target 平台。

对现有代码和 OpenCode `1.18.25` 能力的核实表明，无文件注入是可行的替代方案，但它通过改变验收标准来缩小范围，不能冒充原始方案的等价实现。已确认的 P1 保留 Workspace 文件和必要的安全所有权，复用 Effect worker 的原则与现有协调能力，但不声明 Skill/MCP composite readiness，不建立完整 AgentTarget 平台，也不提前承担 stdio、全生命周期和聚合 UI。保持与 Skills 相同的 eager 时机意味着共享 Agent 的多 Workspace surface 仍应在当前 worker batch 内按 consumer 合并一次 activation；这是 P1 中最大的必要运行时改动，但仍是局部协调，不要求新的持久 aggregate。

因此最终判断是：P1 已消除主要的设计偏移，范围与原始最小闭环一致；剩余开发面较大是物理文件、共享 Agent 与可靠 Effect 收敛三项真实约束共同造成的，不再是为未来平台预支的复杂度。任何把 user-config merge、独立 sidecar、AgentTarget、全局 exactly-once activation、全入口 admission、stdio 或 retirement 加回一期的提议，都应被视为新的范围变更并重新评估，而不能以“完善 P1”为名自动进入实现。
