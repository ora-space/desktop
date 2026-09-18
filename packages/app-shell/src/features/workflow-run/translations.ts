// Pure translation data: safe to compose without importing feature implementation.
export const workflowRunTranslations = {
  "zh-CN": {
    "errors.workflow_no_published_snapshot": "该工作流没有已发布的快照。",
    "errors.workflow_run_cannot_use_draft_snapshot":
      "工作流运行不能使用草稿快照。",
    "errors.workflow_run_not_found": "未找到该工作流运行。",
    "errors.workflow_run_active": "该工作流运行仍处于活动状态。",
    "errors.workflow_run_graph_parse": "工作流图解析失败。",
    "errors.workflow_run_validation": "工作流运行校验失败。",
    "errors.workflow_skill_not_found": "该工作流需要的技能不可用。",
    "errors.workflow_role_not_found": "未找到该工作流角色。",
    "errors.workflow_run_start_failed": "启动工作流运行失败。",
    "errors.workflow_run_not_restartable": "该工作流运行无法重新启动。",
    "errors.workflow_run_not_resumable": "该工作流运行当前无法从失败处继续。",
    "errors.workflow_snapshot_incompatible_with_resume":
      "新版本与本次运行不兼容，无法换版本续跑：{{reason}}",
    "errors.workflow_run_not_editable": "该工作流运行当前不可编辑。",
    "errors.workflow_node_not_found": "未找到该工作流节点。",
    "errors.workflow_node_not_awaiting_input":
      "该节点当前不在等待人工输入，无法完成。",
    "errors.workflow_node_not_diagnosable":
      "只有失败的智能体节点才能做 AI 分析。",
    "workflowRun.loading": "正在加载运行…",
    "workflowRun.placeholderTitle": "工作流运行台",
    "workflowRun.placeholderSubtitle": "运行工作区",
    "workflowRun.placeholderBody":
      "在舞台跟进当前节点，或在全图俯瞰整条执行路径。",
    "workflowRun.field.status": "状态",
    "workflowRun.field.nodes": "节点数",
    "workflowRun.field.progress": "进度",
    "workflowRun.field.currentNode": "当前节点",
    "workflowRun.field.startedAt": "开始",
    "workflowRun.field.finishedAt": "结束",
    "workflowRun.field.fileChanges": "文件改动",
    "workflowRun.progressValue": "{{done}} / {{total}}",
    "workflowRun.currentNodeIdle": "等待开始",
    "workflowRun.currentNodeDone": "已全部完成",
    "workflowRun.currentNodeCancelled": "已取消",
    "workflowRun.cancelAction": "取消运行",
    "workflowRun.startAction": "启动",
    "workflowRun.runAgainAction": "从头重新运行",
    "workflowRun.resumeFromFailure": "从失败处继续",
    "workflowRun.resume.title": "从失败处继续",
    "workflowRun.resume.description":
      "已成功的节点不会重跑。先决定失败节点改过的文件怎么处理：",
    "workflowRun.resume.keep": "保留现状（默认）",
    "workflowRun.resume.nodeFiles": "只回滚失败节点改过的文件",
    "workflowRun.resume.checkpoint": "整体回滚到检查点",
    "workflowRun.resume.nodeSummary":
      "节点 {{nodeId}}：节点记录改动 {{nodeFiles}} 个文件；自检查点以来共 {{total}} 个变化，其中 {{extra}} 个不在节点记录里（可能是失败后手工改的）",
    "workflowRun.resume.reason.no_checkpoint": "该节点没有记录检查点",
    "workflowRun.resume.reason.siblings_ran_after_checkpoint":
      "检查点之后有其他节点跑过，整体回滚会抹掉它们的成果",
    "workflowRun.resume.reason.not_resumable": "当前运行不能续跑",
    "workflowRun.resume.rollbackUnavailable.composite_region":
      "失败发生在迭代节点内部，只能保留现状或整体回滚到迭代开始前的检查点",
    "workflowRun.resume.rollbackUnavailable.no_file_changes":
      "失败节点没有记录可回滚的文件改动",
    "workflowRun.resume.compositeRestart":
      "迭代节点「{{name}}」将从第一轮重新开始",
    "workflowRun.resume.safetyNote": "回滚前会自动再存一个检查点，可以反悔。",
    "workflowRun.resume.loadingPreview": "正在读取改动…",
    "workflowRun.resume.previewFailed": "无法读取改动。",
    "workflowRun.resume.confirm": "从失败处继续",
    "workflowRun.resume.switchPublished":
      "改用当前发布版本 {{version}} 续跑（当前运行用的是 {{current}}）",
    "workflowRun.resume.snapshotReason.node_missing": "新版本删掉了节点 {{id}}",
    "workflowRun.resume.snapshotReason.node_type_changed":
      "节点 {{id}} 的类型变了",
    "workflowRun.resume.snapshotReason.start_node_changed": "开始节点变了",
    "workflowRun.resume.snapshotReason.start_variables_changed":
      "开始节点的输入变量变了",
    "workflowRun.resume.snapshotReason.variable_type_changed":
      "变量 {{id}} 的类型变了",
    "workflowRun.resume.snapshotReason.variable_missing":
      "新版本没有变量 {{id}}",
    "workflowRun.stopAction": "终止",
    "workflowRun.stopTitle": "终止此次运行？",
    "workflowRun.stopDescription":
      "将立即停止“{{name}}”的执行。已完成的节点会保留，未完成的进度无法继续。",
    "workflowRun.stopConfirmAction": "确认终止",
    "workflowRun.stopping": "终止中…",
    "workflowRun.missing": "找不到此次运行。",
    "workflowRun.viewMode.label": "运行视图",
    "workflowRun.viewMode.theater": "舞台",
    "workflowRun.viewMode.overview": "全图",
    "workflowRun.theater.path": "执行路径",
    "workflowRun.theater.topLevelPath": "顶层执行路径",
    "workflowRun.theater.iterationChipSummary":
      "{{members}} 成员 · {{rounds}} 轮",
    "workflowRun.theater.iterationSummary":
      "{{members}} 个成员 · {{rounds}} 轮",
    "workflowRun.theater.iterationMembers": "{{members}} 个成员",
    "workflowRun.theater.iterationRoundProgress": "{{done}}/{{total}} 完成",
    "workflowRun.theater.iterationNavigatorLabel":
      "{{name}}，第 {{round}}/{{total}} 轮",
    "workflowRun.theater.roundPosition": "第 {{round}}/{{total}} 轮",
    "workflowRun.theater.previousRound": "上一轮",
    "workflowRun.theater.nextRound": "下一轮",
    "workflowRun.theater.selectRound": "选择迭代轮次",
    "workflowRun.theater.roundOption": "第 {{round}} 轮",
    "workflowRun.theater.parallelGroup": "并行 {{count}}",
    "workflowRun.theater.conditionalGroup": "条件分支 {{count}}",
    "workflowRun.theater.sequentialGroup": "串行阶段",
    "workflowRun.theater.notRunThisRound": "本轮未执行",
    "workflowRun.theater.executionContext": "节点执行上下文",
    "workflowRun.theater.contextParallel": "并行 {{index}}/{{count}}",
    "workflowRun.theater.contextConditional": "条件分支 {{index}}/{{count}}",
    "workflowRun.theater.contextSequential": "串行阶段",
    "workflowRun.theater.instruction": "指令",
    "workflowRun.theater.empty": "此运行没有可展示的节点。",
    "workflowRun.theater.parallelHint":
      "{{count}} 个节点并行 · 当前 {{index}}/{{count}}，可拖拽或点选切换",
    "workflowRun.theater.parallelSwitch": "切换并行节点",
    "workflowRun.theater.parallelPrev": "上一个并行节点",
    "workflowRun.theater.parallelNext": "下一个并行节点",
    "workflowRun.theater.parallelCount": "{{count}} 并行",
    "workflowRun.theater.focusAct": "聚焦到 {{name}}",
    "workflowRun.theater.parallelDragHint": "左右拖拽切换",
    "workflowRun.theater.returnOverviewHint": "按 Esc 返回全图",
    "workflowRun.theater.inspectorHint": "点击卡片右上角按钮查看阶段详情",
    "workflowRun.theater.hitlHint": "点击卡片右上角按钮查看阶段详情",
    "workflowRun.theater.openInspector": "打开节点详情",
    "workflowRun.conversation.label": "节点会话",
    "workflowRun.conversation.open": "查看节点会话",
    "workflowRun.conversation.backToAct": "返回阶段摘要",
    "workflowRun.conversation.sessionMode": "节点会话",
    "workflowRun.conversation.nodeInput": "节点输入",
    "workflowRun.conversation.agentReply": "Agent",
    "workflowRun.conversation.inputPending":
      "节点开始运行后，输入会显示在这里。",
    "workflowRun.conversation.idle": "Agent 尚未启动",
    "workflowRun.conversation.waiting": "Agent 正在处理",
    "workflowRun.theater.outputPending": "等待输出结果",
    "workflowRun.conversation.empty": "尚无消息",
    "workflowRun.conversation.hiddenActivity": "已隐藏 {{count}} 条过程消息",
    "workflowRun.conversation.userAnchorLabel": "用户消息 {{index}}",
    "workflowRun.conversation.agentAnchorLabel": "Agent 消息 {{index}}",
    "workflowRun.result.title.succeeded": "运行完成",
    "workflowRun.result.title.failed": "运行失败",
    "workflowRun.result.title.cancelled": "已取消",
    "workflowRun.result.body.succeeded":
      "本轮工作流已成功结束。可在路径中回顾各步骤，或从顶栏再跑一次。",
    "workflowRun.result.body.failed":
      "运行在某个步骤失败后结束。打开路径查看失败节点与详情。",
    "workflowRun.result.body.cancelled":
      "运行已停止。已执行的步骤仍可在路径中回顾。",
    "workflowRun.result.showOverview": "查看全图",
    "workflowRun.result.openArtifacts": "打开最近成果",
    "workflowRun.result.pathHint":
      "点路径上的「结果」或步骤可切换回顾；再跑一次请用顶栏按钮。",
    "workflowRun.result.pathChip": "结果",
    "workflowRun.result.finishedToastTitle": "运行已结束",
    "workflowRun.result.finishedToastDescription": "可切到舞台查看结果摘要。",
    "workflowRun.result.finishedToastAction": "查看结果",
    "workflowRun.inspector.label": "阶段详情",
    "workflowRun.inspector.title": "阶段详情",
    "workflowRun.inspector.nodeSuffix": "{{type}} · 只读",
    "workflowRun.inspector.config": "配置",
    "workflowRun.inspector.execution": "执行",
    "workflowRun.inspector.input": "输入",
    "workflowRun.inspector.output": "输出",
    "workflowRun.inspector.ioEmpty": "此步骤尚无运行时输入/输出",
    "workflowRun.inspector.ioExpand": "展开",
    "workflowRun.inspector.ioCollapse": "收起",
    "workflowRun.inspector.selectHint": "与舞台当前步骤同步",
    "workflowRun.inspector.empty": "尚未聚焦步骤",
    "workflowRun.inspector.emptyHint":
      "从上方路径选择一个节点，即可查看其设置与成果。",
    "workflowRun.inspector.collapse": "关闭阶段详情",
    "workflowRun.inspector.expand": "展开阶段详情",
    "workflowRun.inspector.resize":
      "拖拽调整阶段详情宽度；双击恢复默认；拖窄可关闭",
    "workflowRun.inspector.skillOpen": "查看 Skill「{{name}}」简介",
    "workflowRun.inspector.roleOpen": "查看角色「{{name}}」简介",
    "workflowRun.inspector.mcpOpen": "查看 MCP「{{name}}」简介",
    "workflowRun.inspector.textOpen": "查看完整{{field}}",
    "workflowRun.inspector.catalogNoDescription": "暂无简介",
    "workflowRun.inspector.saveDraft": "保存",
    "workflowRun.inspector.savingDraft": "保存中…",
    "workflowRun.inspector.discardDraft": "放弃",
    "workflowRun.loopRounds.title": "循环轮次",
    "workflowRun.loopRounds.round": "第 {{round}} 轮",
    "workflowRun.loopRounds.empty": "循环开始后会在这里显示每轮节点状态。",
    "workflowRun.artifacts.title": "成果",
    "workflowRun.artifacts.countBadge": "{{count}} 个成果",
    "workflowRun.artifacts.empty": "该步骤产出后会显示在这里。",
    "workflowRun.artifacts.kind.text": "文本",
    "workflowRun.artifacts.kind.markdown": "Markdown",
    "workflowRun.artifacts.kind.file": "文件",
    "workflowRun.artifacts.kind.diff": "Diff",
    "workflowRun.overview.label": "工作流全图俯瞰",
    "workflowRun.overview.roundBadge": "第 {{round}} 轮",
    "workflowRun.overview.regionSummary": "循环体节点：{{total}} 个",
    "workflowRun.overview.internalStart": "迭代内部开始",
    "workflowRun.overview.zoomControls": "运行画布视图控制",
    "workflowRun.overview.zoomOut": "缩小运行画布",
    "workflowRun.overview.zoomIn": "放大运行画布",
    "workflowRun.overview.fitView": "显示完整运行图",
    "workflowRun.inspector.rounds": "迭代轮次",
    "workflowRun.overview.hint": "点击节点回到舞台并聚焦该步骤",
    "workflowRun.nodeStatus.idle": "未执行",
    "workflowRun.nodeStatus.inactive": "未激活分支",
    "workflowRun.status.pending": "未运行",
    "workflowRun.status.running": "运行中",
    "workflowRun.status.awaiting_input": "等待参与",
    "workflowRun.completeNode.action": "完成当前节点",
    "workflowRun.completeNode.disabledHint":
      "请等待节点回复完成后再完成当前节点。",
    "workflowRun.hitl.panelLabel": "需要你的参与",
    "workflowRun.hitl.title": "确认本步理解",
    "workflowRun.hitl.hint": "填写后提交，工作流才会继续。",
    "workflowRun.hitl.submit": "提交并继续",
    "workflowRun.hitl.submitting": "提交中…",
    "workflowRun.hitl.required": "请填写此项",
    "workflowRun.hitl.selectPlaceholder": "请选择",
    "workflowRun.hitl.waitingNode": "当前等待：{{name}}",
    "workflowRun.hitl.multiWaiting": "{{count}} 个节点需要你的参与",
    "workflowRun.hitl.multiWaitingHint": "可任选一个节点完成确认",
    "workflowRun.hitl.gatesLabel": "等待参与的节点",
    "workflowRun.hitl.reopenTitle": "需要你的参与",
    "workflowRun.hitl.reopenAction": "打开",
    "workflowRun.hitl.collapseAction": "收起",
    "workflowRun.hitl.chooseHint": "选择一项以继续",
    "workflowRun.hitl.chooseRequired": "请选择「{{label}}」",
    "workflowRun.hitl.composerPlaceholder": "补充说明，Enter 发送…",
    "workflowRun.hitl.choiceOnlyPlaceholder": "在下方选择一项以继续",
    "workflowRun.hitl.toastTitle": "需要你参与",
    "workflowRun.hitl.toastDescription":
      "有节点在等待确认。可在全图找到暖色节点，或切到舞台处理。",
    "workflowRun.hitl.toastClarifyDescription":
      "模型在提问。可在全图找到暖色节点，或切到舞台回答。",
    "workflowRun.hitl.toastAction": "去处理",
    "workflowRun.hitl.sidebarBadge": "待参与",
    "workflowRun.hitl.kind.approval": "审批",
    "workflowRun.hitl.kind.feedback": "反馈",
    "workflowRun.hitl.kind.clarify": "提问",
    "workflowRun.hitl.modelQuestion": "模型提问",
    "workflowRun.hitl.timeoutAt": "截止 {{at}}",
    "workflowRun.status.succeeded": "成功",
    "workflowRun.status.failed": "失败",
    "workflowRun.status.cancelled": "已取消",
    "workflowRun.runPickWorkflow": "请先选择一个工作流。",
    "workflowRun.runName": "运行名称",
    "workflowRun.runNamePlaceholder": "输入运行名称",
    "workflowRun.runNamePlaceholderWithDefault": "默认：{{name}}",
    "workflowRun.runKickoffInput": "本次任务文本",
    "workflowRun.runKickoffInputPlaceholder": "输入本次运行的任务文本",
    "workflowRun.runKickoffInputHint": "留空则使用 Start 节点的默认任务文本。",
    "workflowRun.runInWorkspaceDescription":
      "将在当前工作区中创建“{{name}}”的工作流运行。",
    "workflowRun.createRun": "创建运行",
    "workflowRun.runRequiredName": "请填写运行名称。",
    "workflowRun.injectLastFailure":
      "节点重跑时把上次失败原因告诉智能体",
    "workflowRun.kickoffInput": "启动输入（可选）",
    "workflowRun.kickoffPlaceholder": "例如：审查当前分支的未提交改动",
    "workflowRun.startConfirm": "启动",
    "workflowRun.starting": "启动中…",
    "workflowRun.startFailed": "启动失败。",
    "workflowRun.startInputsDescription":
      "填写 Start 节点声明的变量，然后开始本次运行。",
    "workflowRun.startInputsEmpty": "此工作流没有需要填写的启动变量。",
    "workflowRun.inputOptional": "可选",
    "workflowRun.inputRequired": "必填",
    "workflowRun.inputRequiredError": "请填写此项。",
    "workflowRun.selectPlaceholder": "请选择",
    "workflowRun.cancelFailed": "停止运行失败。",
    "workflowRun.rerunFailed": "重新运行失败。",
    "workflowRun.resumeFailed": "从失败处继续失败。",
    "workflowRun.errorKind.missing_agent_ref": "节点未指定智能体",
    "workflowRun.errorHint.missing_agent_ref":
      "在工作流里给该节点选择一个智能体后发布新版本",
    "workflowRun.errorKind.workflow_model_not_found": "模型不可用",
    "workflowRun.errorHint.workflow_model_not_found":
      "智能体当前不提供该模型，检查智能体配置或稍后重试",
    "workflowRun.errorKind.missing_agent_config": "智能体配置缺失",
    "workflowRun.errorHint.missing_agent_config":
      "检查该智能体是否仍然存在并已配置",
    "workflowRun.errorKind.invalid_run_payload": "运行的冻结数据无效",
    "workflowRun.errorHint.invalid_run_payload":
      "该运行的快照已损坏，请从头重新运行",
    "workflowRun.errorKind.prompt_template": "提示词模板无法渲染",
    "workflowRun.errorHint.prompt_template":
      "修正模板中引用的变量后发布新版本",
    "workflowRun.errorKind.structured_output": "结构化输出不合格",
    "workflowRun.errorHint.structured_output":
      "智能体的回复不符合输出结构；可直接续跑让它带着失败信息重试，或调整提示词/输出结构后发布新版本",
    "workflowRun.errorKind.missing_skill_materialization": "技能未就绪",
    "workflowRun.errorHint.missing_skill_materialization":
      "重新发布工作流以重新生成技能文件",
    "workflowRun.errorKind.session_ended_without_stop_reason": "会话异常结束",
    "workflowRun.errorHint.session_ended_without_stop_reason":
      "通常是临时故障，可直接续跑",
    "workflowRun.errorKind.session_binding_rejected": "会话未能建立",
    "workflowRun.errorHint.session_binding_rejected":
      "通常是临时故障，可直接续跑",
    "workflowRun.errorKind.baseline_persist": "工作区基线保存失败",
    "workflowRun.errorHint.baseline_persist":
      "检查磁盘空间与权限后续跑",
    "workflowRun.errorKind.repository": "数据库操作失败",
    "workflowRun.errorHint.repository": "通常是临时故障，可直接续跑",
    "workflowRun.errorKind.session": "智能体会话失败",
    "workflowRun.errorHint.session": "检查智能体进程与网络后续跑",
    "workflowRun.errorKind.agent_refusal": "智能体拒绝了请求",
    "workflowRun.errorHint.agent_refusal":
      "智能体拒绝了请求；可直接续跑让它带着失败信息重试，或调整提示词后发布新版本",
    "workflowRun.errorKind.unknown_stop_reason": "未知的停止原因",
    "workflowRun.errorHint.unknown_stop_reason":
      "智能体以本版本 Ora 不认识的方式停止，请升级 Ora 或更换智能体",
    "workflowRun.errorKind.interrupted_by_restart": "被应用重启打断",
    "workflowRun.errorHint.interrupted_by_restart":
      "应用重启时该节点仍在运行，可直接续跑",
    "workflowRun.errorKind.multiple_outputs": "多个输出节点同时完成",
    "workflowRun.errorHint.multiple_outputs":
      "工作流结构有误，修正分支后发布新版本",
    "workflowRun.errorKind.condition_evaluation": "条件无法判断",
    "workflowRun.errorHint.condition_evaluation":
      "条件引用的变量缺失或无效，修正后发布新版本",
    "workflowRun.errorAttempt": "第 {{count}} 次尝试",
    "workflowRun.errorNotResumableHint":
      "这类失败通常源于工作流本身，直接续跑很可能再次失败；建议修改工作流后重新运行。",
    "workflowRun.errorInjectedResumeHint":
      "同版本续跑时，Ora 会把这次失败的类型、原因和上次输出告诉智能体让它重试；若仍失败，再修改工作流并发布新版本。",
    "workflowRun.injectedFailure.title": "本次尝试注入的上次失败信息",
    "workflowRun.resumeFromTopHint":
      "可在顶部点「从失败处继续」重跑这个节点",
    "workflowRun.nodeFromOlderSnapshotHint":
      "此节点的结果来自本运行之前使用的版本（续跑时已切换版本）",
    "workflowRun.aiDiagnosis.run": "让 AI 分析",
    "workflowRun.aiDiagnosis.running": "AI 正在分析…",
    "workflowRun.aiDiagnosis.rerun": "重新分析",
    "workflowRun.aiDiagnosis.title": "AI 推测（{{model}}）",
    "workflowRun.aiDiagnosis.disclaimer":
      "这是模型的推测，不参与任何自动判断。",
  },
  "en-US": {
    "errors.workflow_no_published_snapshot":
      "The workflow has no published snapshot.",
    "errors.workflow_run_cannot_use_draft_snapshot":
      "Workflow runs cannot use a draft snapshot.",
    "errors.workflow_run_not_found": "Workflow run not found.",
    "errors.workflow_run_active": "The workflow run is still active.",
    "errors.workflow_run_graph_parse": "Failed to parse the workflow graph.",
    "errors.workflow_run_validation": "Workflow run validation failed.",
    "errors.workflow_skill_not_found":
      "A skill required by this workflow is unavailable.",
    "errors.workflow_role_not_found": "Workflow role not found.",
    "errors.workflow_run_start_failed": "Failed to start the workflow run.",
    "errors.workflow_run_not_restartable":
      "The workflow run cannot be restarted.",
    "errors.workflow_run_not_resumable":
      "The workflow run cannot be resumed from failure right now.",
    "errors.workflow_snapshot_incompatible_with_resume":
      "The new version is incompatible with this run and cannot be used to resume: {{reason}}",
    "errors.workflow_run_not_editable":
      "The workflow run is not editable right now.",
    "errors.workflow_node_not_found": "Workflow node not found.",
    "errors.workflow_node_not_awaiting_input":
      "This node is not awaiting input and cannot be completed.",
    "errors.workflow_node_not_diagnosable":
      "AI analysis is only available for a failed agent node.",
    "workflowRun.loading": "Loading run…",
    "workflowRun.placeholderTitle": "Workflow run workspace",
    "workflowRun.placeholderSubtitle": "Run workspace",
    "workflowRun.placeholderBody":
      "Follow the focused act on Theater, or survey the full path on Overview.",
    "workflowRun.field.status": "Status",
    "workflowRun.field.nodes": "Nodes",
    "workflowRun.field.progress": "Progress",
    "workflowRun.field.currentNode": "Current node",
    "workflowRun.field.startedAt": "Started",
    "workflowRun.field.finishedAt": "Finished",
    "workflowRun.field.fileChanges": "File changes",
    "workflowRun.progressValue": "{{done}} / {{total}}",
    "workflowRun.currentNodeIdle": "Waiting to start",
    "workflowRun.currentNodeDone": "All nodes finished",
    "workflowRun.currentNodeCancelled": "Cancelled",
    "workflowRun.cancelAction": "Cancel run",
    "workflowRun.startAction": "Start",
    "workflowRun.runAgainAction": "Run again from start",
    "workflowRun.resumeFromFailure": "Resume from failure",
    "workflowRun.resume.title": "Resume from failure",
    "workflowRun.resume.description":
      "Succeeded nodes will not run again. First decide what to do with the files the failed nodes changed:",
    "workflowRun.resume.keep": "Keep the worktree as it is (default)",
    "workflowRun.resume.nodeFiles": "Roll back only the files the failed nodes changed",
    "workflowRun.resume.checkpoint": "Roll back everything to the checkpoint",
    "workflowRun.resume.nodeSummary":
      "Node {{nodeId}}: the node recorded {{nodeFiles}} files; {{total}} changes since the checkpoint, {{extra}} of which are not in the node record (possibly edited by hand after the failure)",
    "workflowRun.resume.reason.no_checkpoint":
      "No checkpoint was recorded for this node",
    "workflowRun.resume.reason.siblings_ran_after_checkpoint":
      "Other nodes ran after the checkpoint; a full rollback would erase their work",
    "workflowRun.resume.reason.not_resumable": "This run cannot be resumed",
    "workflowRun.resume.rollbackUnavailable.composite_region":
      "The failure is inside an iteration; keep the worktree or roll back to the checkpoint taken before the iteration started",
    "workflowRun.resume.rollbackUnavailable.no_file_changes":
      "The failed node did not record file changes that can be rolled back",
    "workflowRun.resume.compositeRestart":
      'Iteration node "{{name}}" will restart from its first round',
    "workflowRun.resume.safetyNote":
      "A checkpoint is saved automatically before rollback, so you can undo.",
    "workflowRun.resume.loadingPreview": "Reading changes…",
    "workflowRun.resume.previewFailed": "Could not read the changes.",
    "workflowRun.resume.confirm": "Resume from failure",
    "workflowRun.resume.switchPublished":
      "Resume with the currently published version {{version}} (this run uses {{current}})",
    "workflowRun.resume.snapshotReason.node_missing":
      "The new version removed node {{id}}",
    "workflowRun.resume.snapshotReason.node_type_changed":
      "Node {{id}} changed type",
    "workflowRun.resume.snapshotReason.start_node_changed":
      "The start node changed",
    "workflowRun.resume.snapshotReason.start_variables_changed":
      "The start node's input variables changed",
    "workflowRun.resume.snapshotReason.variable_type_changed":
      "Variable {{id}} changed type",
    "workflowRun.resume.snapshotReason.variable_missing":
      "Variable {{id}} no longer exists",
    "workflowRun.stopAction": "Stop",
    "workflowRun.stopTitle": "Stop this run?",
    "workflowRun.stopDescription":
      "This immediately stops “{{name}}”. Finished nodes stay; unfinished work cannot continue.",
    "workflowRun.stopConfirmAction": "Stop run",
    "workflowRun.stopping": "Stopping…",
    "workflowRun.missing": "This run could not be found.",
    "workflowRun.viewMode.label": "Run view",
    "workflowRun.viewMode.theater": "Theater",
    "workflowRun.viewMode.overview": "Overview",
    "workflowRun.theater.path": "Execution path",
    "workflowRun.theater.topLevelPath": "Top-level execution path",
    "workflowRun.theater.iterationChipSummary":
      "{{members}} members · {{rounds}} rounds",
    "workflowRun.theater.iterationSummary":
      "{{members}} members · {{rounds}} rounds",
    "workflowRun.theater.iterationMembers": "{{members}} members",
    "workflowRun.theater.iterationRoundProgress": "{{done}}/{{total}} complete",
    "workflowRun.theater.iterationNavigatorLabel":
      "{{name}}, round {{round}} of {{total}}",
    "workflowRun.theater.roundPosition": "Round {{round}}/{{total}}",
    "workflowRun.theater.previousRound": "Previous round",
    "workflowRun.theater.nextRound": "Next round",
    "workflowRun.theater.selectRound": "Select iteration round",
    "workflowRun.theater.roundOption": "Round {{round}}",
    "workflowRun.theater.parallelGroup": "{{count}} parallel",
    "workflowRun.theater.conditionalGroup": "{{count}} conditional branches",
    "workflowRun.theater.sequentialGroup": "Sequential stage",
    "workflowRun.theater.notRunThisRound": "Not run this round",
    "workflowRun.theater.executionContext": "Node execution context",
    "workflowRun.theater.contextParallel": "Parallel {{index}}/{{count}}",
    "workflowRun.theater.contextConditional":
      "Conditional branch {{index}}/{{count}}",
    "workflowRun.theater.contextSequential": "Sequential stage",
    "workflowRun.theater.instruction": "Instruction",
    "workflowRun.theater.empty": "This run has no nodes to show.",
    "workflowRun.theater.parallelHint":
      "{{count}} in parallel · {{index}}/{{count}} — drag or pick to switch",
    "workflowRun.theater.parallelSwitch": "Switch parallel nodes",
    "workflowRun.theater.parallelPrev": "Previous parallel node",
    "workflowRun.theater.parallelNext": "Next parallel node",
    "workflowRun.theater.parallelCount": "{{count}} parallel",
    "workflowRun.theater.focusAct": "Focus {{name}}",
    "workflowRun.theater.parallelDragHint": "Drag sideways to switch",
    "workflowRun.theater.returnOverviewHint": "Press Esc for Overview",
    "workflowRun.theater.inspectorHint":
      "Use the button in the card's top-right corner to open act details",
    "workflowRun.theater.hitlHint":
      "Use the button in the card's top-right corner to open act details",
    "workflowRun.theater.openInspector": "Open node details",
    "workflowRun.conversation.label": "Node conversation",
    "workflowRun.conversation.open": "View node conversation",
    "workflowRun.conversation.backToAct": "Return to the act summary",
    "workflowRun.conversation.sessionMode": "Node conversation",
    "workflowRun.conversation.nodeInput": "Node input",
    "workflowRun.conversation.agentReply": "Agent",
    "workflowRun.conversation.inputPending":
      "Input appears here after this node starts.",
    "workflowRun.conversation.idle": "Agent has not started",
    "workflowRun.conversation.waiting": "Agent is working",
    "workflowRun.theater.outputPending": "Waiting for output",
    "workflowRun.conversation.empty": "No messages yet",
    "workflowRun.conversation.hiddenActivity":
      "{{count}} process messages hidden",
    "workflowRun.conversation.userAnchorLabel": "User message {{index}}",
    "workflowRun.conversation.agentAnchorLabel": "Agent message {{index}}",
    "workflowRun.result.title.succeeded": "Run complete",
    "workflowRun.result.title.failed": "Run failed",
    "workflowRun.result.title.cancelled": "Cancelled",
    "workflowRun.result.body.succeeded":
      "This workflow run finished successfully. Review acts on the path, or use Run again in the header.",
    "workflowRun.result.body.failed":
      "The run ended after a step failed. Open the path for the failing node and details.",
    "workflowRun.result.body.cancelled":
      "The run was stopped. Completed steps remain available on the path.",
    "workflowRun.result.showOverview": "Show Overview",
    "workflowRun.result.openArtifacts": "Open recent outcomes",
    "workflowRun.result.pathHint":
      "Use Result or a path chip to switch review; Run again stays in the header.",
    "workflowRun.result.pathChip": "Result",
    "workflowRun.result.finishedToastTitle": "Run finished",
    "workflowRun.result.finishedToastDescription":
      "Open Theater to see the result summary.",
    "workflowRun.result.finishedToastAction": "View result",
    "workflowRun.inspector.label": "Act details",
    "workflowRun.inspector.title": "Act details",
    "workflowRun.inspector.nodeSuffix": "{{type}} · read-only",
    "workflowRun.inspector.config": "Configuration",
    "workflowRun.inspector.execution": "Execution",
    "workflowRun.inspector.input": "Input",
    "workflowRun.inspector.output": "Output",
    "workflowRun.inspector.ioEmpty":
      "No runtime input/output for this step yet",
    "workflowRun.inspector.ioExpand": "Expand",
    "workflowRun.inspector.ioCollapse": "Collapse",
    "workflowRun.inspector.selectHint": "Synced with the focused stage act",
    "workflowRun.inspector.empty": "No act focused",
    "workflowRun.inspector.emptyHint":
      "Pick a step from the path above to see its settings and outcomes.",
    "workflowRun.inspector.collapse": "Close act details",
    "workflowRun.inspector.expand": "Expand act details",
    "workflowRun.inspector.resize":
      "Drag to resize act details; double-click to reset; drag narrow to close",
    "workflowRun.inspector.skillOpen": "View brief for skill “{{name}}”",
    "workflowRun.inspector.roleOpen": "View brief for role “{{name}}”",
    "workflowRun.inspector.mcpOpen": "View brief for MCP “{{name}}”",
    "workflowRun.inspector.textOpen": "View full {{field}}",
    "workflowRun.inspector.catalogNoDescription": "No description available",
    "workflowRun.inspector.saveDraft": "Save",
    "workflowRun.inspector.savingDraft": "Saving…",
    "workflowRun.inspector.discardDraft": "Discard",
    "workflowRun.loopRounds.title": "Loop rounds",
    "workflowRun.loopRounds.round": "Round {{round}}",
    "workflowRun.loopRounds.empty":
      "Per-round node states will appear here after the Loop starts.",
    "workflowRun.artifacts.title": "Outcomes",
    "workflowRun.artifacts.countBadge": "{{count}} outcomes",
    "workflowRun.artifacts.empty":
      "Outputs will appear here when this step produces them.",
    "workflowRun.artifacts.kind.text": "Text",
    "workflowRun.artifacts.kind.markdown": "Markdown",
    "workflowRun.artifacts.kind.file": "File",
    "workflowRun.artifacts.kind.diff": "Diff",
    "workflowRun.overview.label": "Workflow run overview",
    "workflowRun.overview.roundBadge": "Round {{round}}",
    "workflowRun.overview.regionSummary": "Region nodes: {{total}}",
    "workflowRun.overview.internalStart": "Iteration internal start",
    "workflowRun.overview.zoomControls": "Run canvas view controls",
    "workflowRun.overview.zoomOut": "Zoom out run canvas",
    "workflowRun.overview.zoomIn": "Zoom in run canvas",
    "workflowRun.overview.fitView": "Fit complete run graph",
    "workflowRun.inspector.rounds": "Iteration rounds",
    "workflowRun.overview.hint":
      "Click a node to return to Theater focused on that step",
    "workflowRun.nodeStatus.idle": "Idle",
    "workflowRun.nodeStatus.inactive": "Inactive branch",
    "workflowRun.status.pending": "Pending",
    "workflowRun.status.running": "Running",
    "workflowRun.status.awaiting_input": "Awaiting input",
    "workflowRun.completeNode.action": "Complete current node",
    "workflowRun.completeNode.disabledHint":
      "Wait for the node to finish replying before completing it.",
    "workflowRun.hitl.panelLabel": "Your input is required",
    "workflowRun.hitl.title": "Confirm this step",
    "workflowRun.hitl.hint": "Submit to continue the workflow.",
    "workflowRun.hitl.submit": "Submit and continue",
    "workflowRun.hitl.submitting": "Submitting…",
    "workflowRun.hitl.required": "This field is required",
    "workflowRun.hitl.selectPlaceholder": "Select…",
    "workflowRun.hitl.waitingNode": "Waiting on: {{name}}",
    "workflowRun.hitl.multiWaiting": "{{count}} nodes need your input",
    "workflowRun.hitl.multiWaitingHint": "You can answer any waiting node",
    "workflowRun.hitl.gatesLabel": "Waiting nodes",
    "workflowRun.hitl.reopenTitle": "Your input is required",
    "workflowRun.hitl.reopenAction": "Open",
    "workflowRun.hitl.collapseAction": "Collapse",
    "workflowRun.hitl.chooseHint": "Pick an option to continue",
    "workflowRun.hitl.chooseRequired": "Choose “{{label}}”",
    "workflowRun.hitl.composerPlaceholder": "Add a note, Enter to send…",
    "workflowRun.hitl.choiceOnlyPlaceholder":
      "Pick an option below to continue",
    "workflowRun.hitl.toastTitle": "Input needed",
    "workflowRun.hitl.toastDescription":
      "A node is waiting. Find the amber node on Overview, or switch to Theater when ready.",
    "workflowRun.hitl.toastClarifyDescription":
      "The model is asking a question. Find the amber node on Overview, or answer on Theater.",
    "workflowRun.hitl.toastAction": "Review",
    "workflowRun.hitl.sidebarBadge": "Needs you",
    "workflowRun.hitl.kind.approval": "Approval",
    "workflowRun.hitl.kind.feedback": "Feedback",
    "workflowRun.hitl.kind.clarify": "Question",
    "workflowRun.hitl.modelQuestion": "Model question",
    "workflowRun.hitl.timeoutAt": "Due {{at}}",
    "workflowRun.status.succeeded": "Succeeded",
    "workflowRun.status.failed": "Failed",
    "workflowRun.status.cancelled": "Cancelled",
    "workflowRun.runPickWorkflow": "Select a workflow first.",
    "workflowRun.runName": "Run name",
    "workflowRun.runNamePlaceholder": "Enter a run name",
    "workflowRun.runNamePlaceholderWithDefault": "Default: {{name}}",
    "workflowRun.runKickoffInput": "Task text for this run",
    "workflowRun.runKickoffInputPlaceholder":
      "Enter the task text for this run",
    "workflowRun.runKickoffInputHint":
      "Leave empty to use the Start node's default instruction.",
    "workflowRun.runInWorkspaceDescription":
      "Creates a run of “{{name}}” in the current workspace.",
    "workflowRun.createRun": "Create run",
    "workflowRun.runRequiredName": "Enter a run name.",
    "workflowRun.injectLastFailure":
      "Tell the agent why the previous attempt failed when a step runs again",
    "workflowRun.kickoffInput": "Kickoff input (optional)",
    "workflowRun.kickoffPlaceholder":
      "e.g. Review uncommitted changes on this branch",
    "workflowRun.startConfirm": "Start",
    "workflowRun.starting": "Starting…",
    "workflowRun.startFailed": "Failed to start.",
    "workflowRun.startInputsDescription":
      "Fill in the variables declared by the Start node, then begin this run.",
    "workflowRun.startInputsEmpty":
      "This workflow has no Start variables to fill in.",
    "workflowRun.inputOptional": "Optional",
    "workflowRun.inputRequired": "Required",
    "workflowRun.inputRequiredError": "This field is required.",
    "workflowRun.selectPlaceholder": "Select an option",
    "workflowRun.cancelFailed": "Failed to stop the run.",
    "workflowRun.rerunFailed": "Failed to run again.",
    "workflowRun.resumeFailed": "Failed to resume from failure.",
    "workflowRun.errorKind.missing_agent_ref": "Node names no agent",
    "workflowRun.errorHint.missing_agent_ref":
      "Pick an agent for this node and publish a new version",
    "workflowRun.errorKind.workflow_model_not_found": "Model not available",
    "workflowRun.errorHint.workflow_model_not_found":
      "The agent does not advertise this model; check the agent config or retry later",
    "workflowRun.errorKind.missing_agent_config": "Agent configuration missing",
    "workflowRun.errorHint.missing_agent_config":
      "Check that the agent still exists and is configured",
    "workflowRun.errorKind.invalid_run_payload": "Frozen run data invalid",
    "workflowRun.errorHint.invalid_run_payload":
      "The run snapshot is corrupt; run again from start",
    "workflowRun.errorKind.prompt_template": "Prompt template cannot render",
    "workflowRun.errorHint.prompt_template":
      "Fix the variables referenced by the template and publish a new version",
    "workflowRun.errorKind.structured_output": "Structured output invalid",
    "workflowRun.errorHint.structured_output":
      "The agent's reply did not match the output schema; resume to let it retry with the failure context, or adjust the prompt/schema and publish a new version",
    "workflowRun.errorKind.missing_skill_materialization":
      "Skill not materialized",
    "workflowRun.errorHint.missing_skill_materialization":
      "Republish the workflow to regenerate the skill files",
    "workflowRun.errorKind.session_ended_without_stop_reason":
      "Session ended unexpectedly",
    "workflowRun.errorHint.session_ended_without_stop_reason":
      "Usually transient; resume directly",
    "workflowRun.errorKind.session_binding_rejected":
      "Session could not start",
    "workflowRun.errorHint.session_binding_rejected":
      "Usually transient; resume directly",
    "workflowRun.errorKind.baseline_persist":
      "Worktree baseline could not be saved",
    "workflowRun.errorHint.baseline_persist":
      "Check disk space and permissions, then resume",
    "workflowRun.errorKind.repository": "Database operation failed",
    "workflowRun.errorHint.repository": "Usually transient; resume directly",
    "workflowRun.errorKind.session": "Agent session failed",
    "workflowRun.errorHint.session":
      "Check the agent process and network, then resume",
    "workflowRun.errorKind.agent_refusal": "Agent refused the request",
    "workflowRun.errorHint.agent_refusal":
      "The agent refused; resume to let it retry with the failure context, or adjust the prompt and publish a new version",
    "workflowRun.errorKind.unknown_stop_reason": "Unknown stop reason",
    "workflowRun.errorHint.unknown_stop_reason":
      "The agent stopped in a way this Ora version cannot interpret; upgrade Ora or change the agent",
    "workflowRun.errorKind.interrupted_by_restart":
      "Interrupted by app restart",
    "workflowRun.errorHint.interrupted_by_restart":
      "The node was running when the app restarted; resume directly",
    "workflowRun.errorKind.multiple_outputs":
      "Multiple output nodes completed",
    "workflowRun.errorHint.multiple_outputs":
      "The workflow graph is wrong; fix the branches and publish a new version",
    "workflowRun.errorKind.condition_evaluation":
      "Condition could not be evaluated",
    "workflowRun.errorHint.condition_evaluation":
      "A variable used by the condition is missing or invalid; fix it and publish a new version",
    "workflowRun.errorAttempt": "Attempt {{count}}",
    "workflowRun.errorNotResumableHint":
      "This kind of failure usually comes from the workflow itself; resuming as-is will likely fail again. Edit the workflow and run it again.",
    "workflowRun.errorInjectedResumeHint":
      "Resuming on the same version tells the agent this failure's kind, reason and previous output so it can retry; if it still fails, revise the workflow and publish a new version.",
    "workflowRun.injectedFailure.title":
      "Previous-failure context injected into this attempt",
    "workflowRun.resumeFromTopHint":
      "Use “Resume from failure” at the top to rerun this node",
    "workflowRun.nodeFromOlderSnapshotHint":
      "This node's result comes from the version this run used before switching",
    "workflowRun.aiDiagnosis.run": "Ask AI to analyze",
    "workflowRun.aiDiagnosis.running": "AI is analyzing…",
    "workflowRun.aiDiagnosis.rerun": "Analyze again",
    "workflowRun.aiDiagnosis.title": "AI guess ({{model}})",
    "workflowRun.aiDiagnosis.disclaimer":
      "This is a model's guess and drives no automatic decision.",
  },
} as const;
