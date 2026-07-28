import type {
  WorkflowDefinition,
  WorkflowEdge,
  WorkflowLocale,
  WorkflowNode,
} from "./types";

type LocalizedText = {
  zh: string;
  en: string;
};

interface LocalizedNode {
  id: string;
  kind: WorkflowNode["kind"];
  title: LocalizedText;
  description: LocalizedText;
  position: WorkflowNode["position"];
  instruction: LocalizedText;
  model?: string;
  tool?: string;
  condition?: LocalizedText;
  command?: LocalizedText;
  trigger?: string;
  source?: string;
  language?: string;
}

interface LocalizedWorkflow {
  id: string;
  name: LocalizedText;
  description: LocalizedText;
  updatedAt: string;
  nodes: LocalizedNode[];
  edges: Array<Omit<WorkflowEdge, "label"> & { label?: LocalizedText }>;
}

const CODE_REVIEW_WORKFLOW: LocalizedWorkflow = {
  id: "code-review",
  name: { zh: "代码审查工作流", en: "Code review workflow" },
  description: {
    zh: "检查当前分支改动、验证关键路径，并输出带文件定位的审查结论。",
    en: "Inspect branch changes, validate critical paths, and report findings with file locations.",
  },
  updatedAt: "2026-07-28T09:42:00+08:00",
  nodes: [
    {
      id: "start",
      kind: "trigger",
      title: { zh: "开始", en: "Start" },
      description: {
        zh: "接收审查目标、当前分支和工作区状态",
        en: "Receive the review goal, current branch, and workspace state",
      },
      position: { x: 72, y: 286 },
      instruction: {
        zh: "读取用户指定的审查范围；未指定时检查当前工作区的未提交改动，并保留用户已有修改。",
        en: "Use the requested review scope; otherwise inspect uncommitted workspace changes without altering user work.",
      },
      trigger: "Manual",
    },
    {
      id: "understand",
      kind: "llm",
      title: { zh: "理解改动", en: "Understand changes" },
      description: {
        zh: "按模块归纳意图、调用链和影响面",
        en: "Map intent, call paths, and affected modules",
      },
      position: { x: 356, y: 188 },
      instruction: {
        zh: "结合 git diff 与相关实现说明改动要解决的问题，列出受影响的公共 API、数据结构和用户路径。",
        en: "Use the git diff and related code to explain the goal and list affected public APIs, data structures, and user paths.",
      },
      model: "GPT-5",
    },
    {
      id: "quality",
      kind: "condition",
      title: { zh: "质量门禁", en: "Quality gate" },
      description: {
        zh: "按文件类型选择代码验证或文档快审",
        en: "Route source changes to validation and docs to a focused pass",
      },
      position: { x: 650, y: 188 },
      instruction: {
        zh: "若改动包含源代码、构建配置或依赖锁文件，则执行检查；纯文档与资源变更直接进入审查。",
        en: "Run checks for source, build configuration, or lockfile changes; send docs-only and asset-only changes directly to review.",
      },
      condition: {
        zh: "包含源代码、构建配置或依赖变更",
        en: "Contains source, build configuration, or dependency changes",
      },
    },
    {
      id: "tests",
      kind: "code",
      title: { zh: "运行检查", en: "Run checks" },
      description: {
        zh: "执行格式化、静态检查和受影响测试",
        en: "Run formatting, static analysis, and affected tests",
      },
      position: { x: 938, y: 92 },
      instruction: {
        zh: "先运行仓库约定的格式化和静态检查，再运行覆盖改动模块的最小测试集；记录命令、退出码和失败摘要。",
        en: "Run repository formatting and static checks, then the smallest test set covering changed modules; record commands, exit codes, and failures.",
      },
      tool: "Terminal",
      language: "Shell",
      command: {
        zh: "cargo fmt --all -- --check\ntask test",
        en: "cargo fmt --all -- --check\ntask test",
      },
    },
    {
      id: "review",
      kind: "llm",
      title: { zh: "审查 Agent", en: "Review agent" },
      description: {
        zh: "核对正确性、边界条件与回归风险",
        en: "Check correctness, edge cases, and regression risk",
      },
      position: { x: 938, y: 330 },
      instruction: {
        zh: "仅报告可复现且由本次改动引入的问题。按严重程度排序，每项包含文件与行号、触发条件、影响和最小修复建议。",
        en: "Report only reproducible issues introduced by this change. Sort by severity and include file, line, trigger, impact, and minimal remediation.",
      },
      model: "GPT-5",
    },
    {
      id: "output",
      kind: "output",
      title: { zh: "输出报告", en: "Output report" },
      description: {
        zh: "生成可直接用于合并决策的审查摘要",
        en: "Produce a review summary suitable for a merge decision",
      },
      position: { x: 1218, y: 330 },
      instruction: {
        zh: "输出变更摘要、按严重程度排列的问题、已执行验证和剩余风险；没有发现时明确说明。",
        en: "Return a change summary, severity-ranked findings, completed validation, and residual risk; explicitly say when no findings exist.",
      },
    },
  ],
  edges: [
    { id: "e-start-understand", source: "start", target: "understand" },
    { id: "e-understand-quality", source: "understand", target: "quality" },
    {
      id: "e-quality-tests",
      source: "quality",
      target: "tests",
      label: { zh: "需要检查", en: "Checks required" },
    },
    {
      id: "e-quality-review",
      source: "quality",
      target: "review",
      label: { zh: "仅文档", en: "Documentation only" },
    },
    { id: "e-tests-review", source: "tests", target: "review" },
    { id: "e-review-output", source: "review", target: "output" },
  ],
};

const RELEASE_READINESS_WORKFLOW: LocalizedWorkflow = {
  id: "release-readiness",
  name: { zh: "发布准备检查", en: "Release readiness" },
  description: {
    zh: "汇总版本变更、验证发布门禁，并给出可发布或暂缓的明确建议。",
    en: "Summarize release changes, verify gates, and recommend ship or hold.",
  },
  updatedAt: "2026-07-27T17:18:00+08:00",
  nodes: [
    {
      id: "release-input",
      kind: "trigger",
      title: { zh: "锁定发布范围", en: "Lock release scope" },
      description: {
        zh: "接收版本号、目标分支与部署环境",
        en: "Receive version, target branch, and environment",
      },
      position: { x: 72, y: 250 },
      instruction: {
        zh: "确认候选版本号、release 分支、目标环境和计划发布时间；缺失信息需要在报告中标记为阻塞项。",
        en: "Confirm candidate version, release branch, environment, and planned window; flag missing information as blocking.",
      },
      trigger: "Manual",
    },
    {
      id: "release-notes",
      kind: "llm",
      title: { zh: "整理版本变更", en: "Compile release changes" },
      description: {
        zh: "从提交与合并记录生成用户可读变更说明",
        en: "Create user-facing notes from commits and merged changes",
      },
      position: { x: 350, y: 120 },
      instruction: {
        zh: "比较上一个稳定标签与候选版本，按新功能、修复、破坏性变更和运维事项分组，关联对应 issue 或 PR。",
        en: "Compare the previous stable tag with the candidate and group features, fixes, breaking changes, and operations notes with issue or PR links.",
      },
      model: "GPT-5",
    },
    {
      id: "release-validate",
      kind: "code",
      title: { zh: "执行发布验证", en: "Run release validation" },
      description: {
        zh: "运行构建、测试与产物完整性检查",
        en: "Run build, tests, and artifact integrity checks",
      },
      position: { x: 350, y: 374 },
      instruction: {
        zh: "按发布配置执行全量测试和生产构建，校验迁移脚本、制品哈希与版本元数据，并保存失败命令的最后 100 行日志。",
        en: "Run the full suite and production build, validate migrations, artifact hashes, and version metadata, and retain the last 100 log lines for failures.",
      },
      tool: "Terminal",
      language: "Shell",
      command: {
        zh: "git describe --tags --abbrev=0\ntask test\npnpm -r build",
        en: "git describe --tags --abbrev=0\ntask test\npnpm -r build",
      },
    },
    {
      id: "release-gate",
      kind: "condition",
      title: { zh: "发布门禁", en: "Release gate" },
      description: {
        zh: "检查阻塞缺陷、测试结果和回滚条件",
        en: "Evaluate blockers, test results, and rollback readiness",
      },
      position: { x: 650, y: 250 },
      instruction: {
        zh: "只有在关键测试通过、无 P0/P1 未解决缺陷、迁移可回滚且监控告警已配置时进入批准路径。",
        en: "Approve only when critical tests pass, no P0/P1 issue remains, migrations are reversible, and monitoring alerts are configured.",
      },
      condition: {
        zh: "所有强制门禁均通过",
        en: "All mandatory release gates pass",
      },
    },
    {
      id: "release-plan",
      kind: "llm",
      title: { zh: "生成发布清单", en: "Generate release checklist" },
      description: {
        zh: "编排部署、冒烟验证与回滚步骤",
        en: "Sequence deployment, smoke checks, and rollback",
      },
      position: { x: 936, y: 122 },
      instruction: {
        zh: "按时间顺序列出负责人、执行命令、成功信号、观察时长与回滚触发器，避免使用“视情况而定”等模糊描述。",
        en: "List owner, command, success signal, observation window, and rollback trigger in order; avoid ambiguous conditional language.",
      },
      model: "GPT-5",
    },
    {
      id: "release-hold",
      kind: "llm",
      title: { zh: "分析阻塞项", en: "Analyze release blockers" },
      description: {
        zh: "汇总失败原因并提出最短解阻路径",
        en: "Summarize failures and the shortest path to unblock",
      },
      position: { x: 936, y: 378 },
      instruction: {
        zh: "区分必须修复与可接受风险，为每个阻塞项指定负责人、复验方式和建议完成时间，不得弱化失败门禁。",
        en: "Separate must-fix items from accepted risks and assign owner, recheck method, and target time without weakening failed gates.",
      },
      model: "GPT-5",
    },
    {
      id: "release-decision",
      kind: "output",
      title: { zh: "发布决策", en: "Release decision" },
      description: {
        zh: "输出 GO / HOLD 结论与审计依据",
        en: "Return a GO or HOLD decision with evidence",
      },
      position: { x: 1230, y: 250 },
      instruction: {
        zh: "首行输出 GO 或 HOLD，随后列出版本范围、门禁证据、已知风险、执行清单或解阻行动。",
        en: "Start with GO or HOLD, followed by release scope, gate evidence, known risks, and either the rollout checklist or unblock actions.",
      },
    },
  ],
  edges: [
    { id: "e-release-input-notes", source: "release-input", target: "release-notes" },
    { id: "e-release-input-validate", source: "release-input", target: "release-validate" },
    { id: "e-release-notes-gate", source: "release-notes", target: "release-gate" },
    { id: "e-release-validate-gate", source: "release-validate", target: "release-gate" },
    {
      id: "e-release-gate-plan",
      source: "release-gate",
      target: "release-plan",
      label: { zh: "通过", en: "Pass" },
    },
    {
      id: "e-release-gate-hold",
      source: "release-gate",
      target: "release-hold",
      label: { zh: "未通过", en: "Failed" },
    },
    { id: "e-release-plan-decision", source: "release-plan", target: "release-decision" },
    { id: "e-release-hold-decision", source: "release-hold", target: "release-decision" },
  ],
};

const ISSUE_TRIAGE_WORKFLOW: LocalizedWorkflow = {
  id: "issue-triage",
  name: { zh: "问题分类助手", en: "Issue triage assistant" },
  description: {
    zh: "规范化缺陷报告、评估用户影响，并路由到事故响应或产品待办。",
    en: "Normalize bug reports, assess user impact, and route to incident response or backlog.",
  },
  updatedAt: "2026-07-26T14:06:00+08:00",
  nodes: [
    {
      id: "triage-intake",
      kind: "trigger",
      title: { zh: "接收问题", en: "Receive report" },
      description: {
        zh: "采集现象、版本、环境与联系方式",
        en: "Capture symptoms, version, environment, and reporter",
      },
      position: { x: 72, y: 260 },
      instruction: {
        zh: "保留原始描述与附件引用，提取产品版本、操作系统、首次发生时间、影响用户数和可联系的报告人。",
        en: "Preserve the original report and attachment references; extract product version, OS, first occurrence, affected users, and contact.",
      },
      trigger: "Webhook",
    },
    {
      id: "triage-normalize",
      kind: "llm",
      title: { zh: "规范化报告", en: "Normalize report" },
      description: {
        zh: "整理标题、复现步骤、预期与实际结果",
        en: "Structure title, reproduction, expected, and actual results",
      },
      position: { x: 352, y: 138 },
      instruction: {
        zh: "生成一个可搜索的短标题，并将上下文整理为前置条件、最小复现步骤、预期结果、实际结果和发生频率；不得臆造缺失信息。",
        en: "Create a searchable title and structure context as prerequisites, minimal steps, expected result, actual result, and frequency without inventing missing data.",
      },
      model: "GPT-5",
    },
    {
      id: "triage-duplicates",
      kind: "data-source",
      title: { zh: "查找相似问题", en: "Find related issues" },
      description: {
        zh: "搜索已有 issue、已知问题与近期回归",
        en: "Search existing issues, known problems, and recent regressions",
      },
      position: { x: 352, y: 388 },
      instruction: {
        zh: "使用标题关键词、错误码和堆栈关键帧搜索开放与最近关闭的问题，返回最高相关的 3 个候选及差异。",
        en: "Search open and recently closed issues using title terms, error codes, and key stack frames; return the top three candidates and differences.",
      },
      tool: "GitHub",
      source: "GitHub",
      command: {
        zh: "gh issue list --state all --search \"<错误码或关键帧>\" --limit 20",
        en: "gh issue list --state all --search \"<error code or key frame>\" --limit 20",
      },
    },
    {
      id: "triage-severity",
      kind: "condition",
      title: { zh: "影响分级", en: "Impact gate" },
      description: {
        zh: "按可用性、数据风险和影响范围分流",
        en: "Route by availability, data risk, and blast radius",
      },
      position: { x: 654, y: 260 },
      instruction: {
        zh: "若存在数据丢失、安全风险、核心路径完全不可用或影响超过 10% 活跃用户，则进入紧急响应；其余进入常规分诊。",
        en: "Escalate for data loss, security risk, complete failure of a critical path, or impact above 10% of active users; otherwise use normal triage.",
      },
      condition: {
        zh: "达到 P0 / P1 影响阈值",
        en: "Meets P0 or P1 impact threshold",
      },
    },
    {
      id: "triage-incident",
      kind: "llm",
      title: { zh: "启动事故响应", en: "Start incident response" },
      description: {
        zh: "生成告警摘要、缓解动作与升级路径",
        en: "Prepare alert summary, mitigation, and escalation path",
      },
      position: { x: 944, y: 130 },
      instruction: {
        zh: "生成事故标题、严重级别、当前影响、已知时间线、立即缓解动作和需要通知的值班角色；未知项明确标记待确认。",
        en: "Prepare incident title, severity, current impact, known timeline, immediate mitigation, and on-call roles to notify; mark unknowns explicitly.",
      },
      model: "GPT-5",
    },
    {
      id: "triage-backlog",
      kind: "llm",
      title: { zh: "分配产品待办", en: "Assign product backlog" },
      description: {
        zh: "建议优先级、组件、标签与负责人",
        en: "Recommend priority, component, labels, and owner",
      },
      position: { x: 944, y: 390 },
      instruction: {
        zh: "基于用户影响、发生频率、替代方案和修复复杂度给出 P2/P3 优先级，匹配组件负责人，并列出仍需追问的信息。",
        en: "Recommend P2/P3 using user impact, frequency, workaround, and fix complexity; match a component owner and list follow-up questions.",
      },
      model: "GPT-5",
    },
    {
      id: "triage-output",
      kind: "output",
      title: { zh: "输出分诊卡片", en: "Output triage card" },
      description: {
        zh: "生成可直接写入 issue 系统的结构化结果",
        en: "Produce a structured result ready for the issue tracker",
      },
      position: { x: 1236, y: 260 },
      instruction: {
        zh: "输出规范标题、严重级别、组件、标签、疑似重复项、负责人、下一步和证据链接；保留原问题描述引用。",
        en: "Return normalized title, severity, component, labels, possible duplicate, owner, next action, and evidence links while preserving the original report reference.",
      },
    },
  ],
  edges: [
    { id: "e-triage-intake-normalize", source: "triage-intake", target: "triage-normalize" },
    { id: "e-triage-intake-duplicates", source: "triage-intake", target: "triage-duplicates" },
    { id: "e-triage-normalize-severity", source: "triage-normalize", target: "triage-severity" },
    { id: "e-triage-duplicates-severity", source: "triage-duplicates", target: "triage-severity" },
    {
      id: "e-triage-severity-incident",
      source: "triage-severity",
      target: "triage-incident",
      label: { zh: "紧急", en: "Urgent" },
    },
    {
      id: "e-triage-severity-backlog",
      source: "triage-severity",
      target: "triage-backlog",
      label: { zh: "常规", en: "Normal" },
    },
    { id: "e-triage-incident-output", source: "triage-incident", target: "triage-output" },
    { id: "e-triage-backlog-output", source: "triage-backlog", target: "triage-output" },
  ],
};

const OPENSPEC_WORKFLOW: LocalizedWorkflow = {
  id: "openspec-change",
  name: { zh: "OpenSpec 模式", en: "OpenSpec mode" },
  description: {
    zh: "按 OpenSpec 核心流程串联探索、提案、实施、规格同步与归档。",
    en: "Connect the core OpenSpec stages: explore, propose, apply, sync, and archive.",
  },
  updatedAt: "2026-07-28T11:52:00+08:00",
  nodes: [
    {
      id: "openspec-request",
      kind: "trigger",
      title: { zh: "开始变更", en: "Start change" },
      description: {
        zh: "接收需求并确认当前工作目录",
        en: "Receive the request and confirm the working directory",
      },
      position: { x: 56, y: 260 },
      instruction: {
        zh: "提取变更目标、范围与验收信号；所有 OpenSpec 产物和实现都必须保存在当前工作目录。",
        en: "Extract the goal, scope, and acceptance signals; keep every OpenSpec artifact and implementation in the current working directory.",
      },
      trigger: "Manual",
      command: { zh: "pwd", en: "pwd" },
    },
    {
      id: "openspec-explore",
      kind: "llm",
      title: { zh: "探索需求", en: "Explore" },
      description: {
        zh: "梳理现状、边界与关键取舍",
        en: "Clarify current behavior, boundaries, and trade-offs",
      },
      position: { x: 330, y: 260 },
      instruction: {
        zh: "使用 openspec-explore skill 检查相关代码与现有规格，澄清用户路径和边界条件。此阶段只探索，不写实现。",
        en: "Use openspec-explore to inspect related code and specs and clarify user paths and edge cases. Explore only; do not implement.",
      },
      model: "GPT-5",
      command: { zh: "$openspec-explore", en: "$openspec-explore" },
    },
    {
      id: "openspec-propose",
      kind: "llm",
      title: { zh: "创建提案", en: "Propose" },
      description: {
        zh: "生成 proposal、specs、design 与 tasks",
        en: "Create proposal, specs, design, and tasks",
      },
      position: { x: 604, y: 260 },
      instruction: {
        zh: "使用 openspec-propose skill 创建完整 change，确保规格场景可验证、设计决策有依据、任务可以逐项执行。",
        en: "Use openspec-propose to create a complete change with testable scenarios, justified design decisions, and executable tasks.",
      },
      model: "GPT-5",
      command: { zh: "$openspec-propose", en: "$openspec-propose" },
    },
    {
      id: "openspec-apply",
      kind: "llm",
      title: { zh: "实施变更", en: "Apply" },
      description: {
        zh: "按 tasks 实现并完成验证",
        en: "Implement tasks and complete validation",
      },
      position: { x: 878, y: 260 },
      instruction: {
        zh: "使用 openspec-apply-change skill 逐项完成 tasks，补充必要测试并运行仓库要求的验证命令。",
        en: "Use openspec-apply-change to complete tasks in order, add required tests, and run repository validation.",
      },
      model: "GPT-5",
      command: { zh: "$openspec-apply-change", en: "$openspec-apply-change" },
    },
    {
      id: "openspec-sync",
      kind: "llm",
      title: { zh: "同步主规格", en: "Sync main specs" },
      description: {
        zh: "把 delta specs 合并到主规格",
        en: "Merge delta specs into the main specs",
      },
      position: { x: 1152, y: 260 },
      instruction: {
        zh: "使用 openspec-sync-specs skill 同步 delta specs，保留本次变更未触及的既有需求。",
        en: "Use openspec-sync-specs to merge delta specs while preserving existing requirements outside this change.",
      },
      model: "GPT-5",
      command: { zh: "$openspec-sync-specs", en: "$openspec-sync-specs" },
    },
    {
      id: "openspec-archive",
      kind: "tool",
      title: { zh: "归档变更", en: "Archive change" },
      description: {
        zh: "归档已完成的 OpenSpec change",
        en: "Archive the completed OpenSpec change",
      },
      position: { x: 1426, y: 260 },
      instruction: {
        zh: "使用 openspec-archive-change skill 归档已完成的 change，并返回归档名称和最终状态。",
        en: "Use openspec-archive-change to archive the completed change and return its name and final status.",
      },
      tool: "File system",
      command: { zh: "$openspec-archive-change", en: "$openspec-archive-change" },
    },
    {
      id: "openspec-summary",
      kind: "output",
      title: { zh: "输出变更摘要", en: "Output change summary" },
      description: {
        zh: "汇总实现、测试、规格与归档位置",
        en: "Summarize implementation, tests, specs, and archive",
      },
      position: { x: 1700, y: 260 },
      instruction: {
        zh: "输出 change 名称、关键设计决策、完成的 tasks、测试命令与结果、同步的主规格及归档路径。",
        en: "Return the change name, key design decisions, completed tasks, test commands and results, synced main specs, and archive path.",
      },
    },
  ],
  edges: [
    { id: "e-openspec-request-explore", source: "openspec-request", target: "openspec-explore" },
    { id: "e-openspec-explore-propose", source: "openspec-explore", target: "openspec-propose" },
    { id: "e-openspec-propose-apply", source: "openspec-propose", target: "openspec-apply" },
    { id: "e-openspec-apply-sync", source: "openspec-apply", target: "openspec-sync" },
    { id: "e-openspec-sync-archive", source: "openspec-sync", target: "openspec-archive" },
    { id: "e-openspec-archive-summary", source: "openspec-archive", target: "openspec-summary" },
  ],
};

const CI_RECOVERY_WORKFLOW: LocalizedWorkflow = {
  id: "ci-recovery",
  name: { zh: "CI 失败修复", en: "CI failure recovery" },
  description: {
    zh: "定位失败检查、区分环境问题与代码回归，并验证最小修复。",
    en: "Inspect failed checks, separate infrastructure issues from regressions, and verify a minimal fix.",
  },
  updatedAt: "2026-07-28T10:48:00+08:00",
  nodes: [
    {
      id: "ci-input",
      kind: "trigger",
      title: { zh: "接收失败检查", en: "Receive failed check" },
      description: { zh: "获取 PR、工作流与失败 job", en: "Capture PR, workflow, and failed job" },
      position: { x: 72, y: 260 },
      instruction: {
        zh: "确认仓库、PR 编号、失败工作流名称和最近一次 run id，避免分析过期日志。",
        en: "Confirm repository, PR number, failed workflow, and latest run id to avoid stale logs.",
      },
      trigger: "Webhook",
      command: {
        zh: "gh pr checks <PR编号>",
        en: "gh pr checks <PR number>",
      },
    },
    {
      id: "ci-logs",
      kind: "data-source",
      title: { zh: "拉取失败日志", en: "Fetch failure logs" },
      description: { zh: "下载失败步骤与注解", en: "Download failed steps and annotations" },
      position: { x: 350, y: 260 },
      instruction: {
        zh: "只读取失败 job 的日志，保留首个根因错误及前后上下文，不被后续级联错误干扰。",
        en: "Read only failed job logs and retain the first root-cause error with context, ignoring cascade failures.",
      },
      tool: "GitHub",
      source: "GitHub",
      command: {
        zh: "gh run view <run-id> --log-failed",
        en: "gh run view <run-id> --log-failed",
      },
    },
    {
      id: "ci-classify",
      kind: "condition",
      title: { zh: "失败类型", en: "Failure type" },
      description: { zh: "判断偶发基础设施故障或确定性回归", en: "Route flaky infrastructure and deterministic regressions" },
      position: { x: 630, y: 260 },
      instruction: {
        zh: "网络超时、runner 中断或外部服务 5xx 进入重试路径；可稳定复现的编译、测试和 lint 失败进入修复路径。",
        en: "Route network timeouts, runner interruption, and external 5xx errors to retry; route reproducible build, test, and lint failures to repair.",
      },
      condition: {
        zh: "日志显示基础设施或偶发错误",
        en: "Logs indicate infrastructure or flaky failure",
      },
    },
    {
      id: "ci-retry",
      kind: "tool",
      title: { zh: "重跑失败任务", en: "Rerun failed jobs" },
      description: { zh: "仅重试失败 job 并观察结果", en: "Retry failed jobs only and monitor results" },
      position: { x: 916, y: 116 },
      instruction: {
        zh: "只重跑失败 job；若相同错误再次出现则停止重试并转为代码问题。",
        en: "Rerun failed jobs only; if the same error repeats, stop retrying and treat it as a code issue.",
      },
      tool: "GitHub",
      command: {
        zh: "gh run rerun <run-id> --failed",
        en: "gh run rerun <run-id> --failed",
      },
    },
    {
      id: "ci-fix",
      kind: "llm",
      title: { zh: "生成最小修复", en: "Create minimal fix" },
      description: { zh: "复现根因并修改受影响代码", en: "Reproduce the root cause and patch affected code" },
      position: { x: 916, y: 394 },
      instruction: {
        zh: "在本地运行失败命令，定位本次改动引入的根因，只修改必要文件，并为回归补充覆盖测试。",
        en: "Run the failing command locally, identify the regression introduced by the change, modify only required files, and add coverage.",
      },
      model: "GPT-5",
      command: {
        zh: "<复制日志中的失败命令>",
        en: "<copy the failing command from logs>",
      },
    },
    {
      id: "ci-result",
      kind: "output",
      title: { zh: "输出修复报告", en: "Output recovery report" },
      description: { zh: "记录根因、操作与验证证据", en: "Record root cause, action, and verification evidence" },
      position: { x: 1208, y: 260 },
      instruction: {
        zh: "输出失败检查、首个根因、是否重试、修改文件、验证命令和剩余风险。",
        en: "Return the failed check, first root cause, retry decision, changed files, validation command, and residual risk.",
      },
    },
  ],
  edges: [
    { id: "e-ci-input-logs", source: "ci-input", target: "ci-logs" },
    { id: "e-ci-logs-classify", source: "ci-logs", target: "ci-classify" },
    { id: "e-ci-classify-retry", source: "ci-classify", target: "ci-retry", label: { zh: "偶发", en: "Flaky" } },
    { id: "e-ci-classify-fix", source: "ci-classify", target: "ci-fix", label: { zh: "回归", en: "Regression" } },
    { id: "e-ci-retry-result", source: "ci-retry", target: "ci-result" },
    { id: "e-ci-fix-result", source: "ci-fix", target: "ci-result" },
  ],
};

const DEPENDENCY_UPDATE_WORKFLOW: LocalizedWorkflow = {
  id: "dependency-update",
  name: { zh: "依赖安全升级", en: "Dependency security update" },
  description: {
    zh: "评估安全公告、升级最小依赖集合，并验证锁文件与关键路径。",
    en: "Assess advisories, update the smallest dependency set, and validate lockfiles and critical paths.",
  },
  updatedAt: "2026-07-27T15:32:00+08:00",
  nodes: [
    {
      id: "deps-scan",
      kind: "code",
      title: { zh: "扫描依赖风险", en: "Scan dependency risk" },
      description: { zh: "检查 Rust 与前端安全公告", en: "Check Rust and frontend advisories" },
      position: { x: 72, y: 260 },
      instruction: {
        zh: "扫描生产与开发依赖，记录公告编号、受影响版本、修复版本和是否可达。",
        en: "Scan production and development dependencies and record advisory, affected range, patched version, and reachability.",
      },
      tool: "Terminal",
      language: "Shell",
      command: {
        zh: "cargo audit\npnpm audit --prod",
        en: "cargo audit\npnpm audit --prod",
      },
    },
    {
      id: "deps-plan",
      kind: "llm",
      title: { zh: "规划最小升级", en: "Plan minimal update" },
      description: { zh: "确定直接依赖、传递依赖与兼容风险", en: "Determine direct, transitive, and compatibility impact" },
      position: { x: 352, y: 260 },
      instruction: {
        zh: "优先选择修复漏洞的最小兼容版本；若必须跨主版本，列出 API 破坏点、迁移步骤与回退方案。",
        en: "Prefer the smallest compatible patched version; for major updates, list API breaks, migration steps, and rollback.",
      },
      model: "GPT-5",
      command: {
        zh: "cargo tree -i <crate>\npnpm why <package>",
        en: "cargo tree -i <crate>\npnpm why <package>",
      },
    },
    {
      id: "deps-update",
      kind: "code",
      title: { zh: "更新锁文件", en: "Update lockfiles" },
      description: { zh: "只更新目标依赖及必要传递项", en: "Update only the target and required transitive packages" },
      position: { x: 632, y: 260 },
      instruction: {
        zh: "限制更新范围，检查 lockfile diff，避免无关依赖漂移或 registry 来源变化。",
        en: "Constrain update scope and inspect lockfile diffs for unrelated drift or registry source changes.",
      },
      tool: "Terminal",
      language: "Shell",
      command: {
        zh: "cargo update -p <crate> --precise <version>\npnpm update <package>@<version> --lockfile-only",
        en: "cargo update -p <crate> --precise <version>\npnpm update <package>@<version> --lockfile-only",
      },
    },
    {
      id: "deps-verify",
      kind: "code",
      title: { zh: "验证兼容性", en: "Verify compatibility" },
      description: { zh: "运行审计、测试和重复依赖检查", en: "Run audit, tests, and duplicate checks" },
      position: { x: 912, y: 260 },
      instruction: {
        zh: "重新运行安全扫描与完整测试，检查重复版本和产物体积变化，确认漏洞已消除且行为未回归。",
        en: "Rerun security scans and the full suite, inspect duplicates and bundle changes, and confirm the advisory is resolved without regressions.",
      },
      tool: "Terminal",
      language: "Shell",
      command: {
        zh: "cargo audit\ncargo tree --duplicates\ntask test",
        en: "cargo audit\ncargo tree --duplicates\ntask test",
      },
    },
    {
      id: "deps-output",
      kind: "output",
      title: { zh: "输出升级摘要", en: "Output update summary" },
      description: { zh: "记录公告、版本差异与验证结果", en: "Record advisories, version changes, and validation" },
      position: { x: 1192, y: 260 },
      instruction: {
        zh: "输出漏洞编号、旧版与新版、受影响锁文件、兼容性处理、验证命令和残余风险。",
        en: "Return advisory, old and new versions, affected lockfiles, compatibility work, validation commands, and residual risk.",
      },
    },
  ],
  edges: [
    { id: "e-deps-scan-plan", source: "deps-scan", target: "deps-plan" },
    { id: "e-deps-plan-update", source: "deps-plan", target: "deps-update" },
    { id: "e-deps-update-verify", source: "deps-update", target: "deps-verify" },
    { id: "e-deps-verify-output", source: "deps-verify", target: "deps-output" },
  ],
};

const WORKFLOWS: readonly LocalizedWorkflow[] = [
  OPENSPEC_WORKFLOW,
  CODE_REVIEW_WORKFLOW,
  CI_RECOVERY_WORKFLOW,
  RELEASE_READINESS_WORKFLOW,
  ISSUE_TRIAGE_WORKFLOW,
  DEPENDENCY_UPDATE_WORKFLOW,
];

/** Localizes a workflow without duplicating graph structure between language fixtures. */
function localizeWorkflow(
  workflow: LocalizedWorkflow,
  locale: WorkflowLocale,
): WorkflowDefinition {
  const language = locale === "zh-CN" ? "zh" : "en";
  return {
    id: workflow.id,
    name: workflow.name[language],
    description: workflow.description[language],
    updatedAt: workflow.updatedAt,
    nodes: workflow.nodes.map((node) => ({
      id: node.id,
      kind: node.kind,
      title: node.title[language],
      description: node.description[language],
      position: node.position,
      config: {
        instruction: node.instruction[language],
        ...(node.model === undefined ? {} : { model: node.model }),
        ...(node.tool === undefined ? {} : { tool: node.tool }),
        ...(node.condition === undefined ? {} : { condition: node.condition[language] }),
        ...(node.command === undefined ? {} : { command: node.command[language] }),
        ...(node.trigger === undefined ? {} : { trigger: node.trigger }),
        ...(node.source === undefined ? {} : { source: node.source }),
        ...(node.language === undefined ? {} : { language: node.language }),
      },
    })),
    edges: workflow.edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      ...(edge.label === undefined ? {} : { label: edge.label[language] }),
    })),
  };
}

export const MOCK_WORKFLOW: WorkflowDefinition = localizeWorkflow(
  OPENSPEC_WORKFLOW,
  "zh-CN",
);

/** Creates the primary localized fixture while preserving stable graph identifiers. */
export function createMockWorkflow(locale: WorkflowLocale): WorkflowDefinition {
  return localizeWorkflow(OPENSPEC_WORKFLOW, locale);
}

/** Provides distinct production-shaped mock graphs for workflow management and editing. */
export function createMockWorkflows(locale: WorkflowLocale): WorkflowDefinition[] {
  return WORKFLOWS.map((workflow) => localizeWorkflow(workflow, locale));
}
