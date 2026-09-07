use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_application::{AgentOutputContract, WorkflowGraph, WorkflowGraphNode};
use ora_contracts::WorkflowRunLocale;
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use std::path::PathBuf;

/// One required skill already resolved from the run's frozen materialization receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequiredWorkflowSkill {
    pub(crate) invocation_name: String,
    pub(crate) package_paths: Vec<PathBuf>,
}

/// Inputs required to turn one workflow node into a self-contained Agent prompt.
pub(crate) struct WorkflowPromptRequest<'a> {
    pub(crate) node: &'a WorkflowGraphNode,
    pub(crate) worktree_root: &'a Path,
    pub(crate) role_content: Option<&'a str>,
    pub(crate) graph_json: &'a str,
    pub(crate) run_input: Option<&'a str>,
    pub(crate) node_runs: &'a [WorkflowNodeRun],
    pub(crate) required_skills: &'a [RequiredWorkflowSkill],
    pub(crate) locale: WorkflowRunLocale,
}

/// Builds a structured prompt that identifies the current step and its workflow context.
pub(crate) fn assemble_workflow_prompt(request: WorkflowPromptRequest<'_>) -> Vec<ContentBlock> {
    let required_skills = render_required_skills(request.required_skills, request.locale);
    let mut blocks = Vec::new();

    if !required_skills.is_empty() {
        // The leading slash commands perform the actual CLI invocation; keeping their mandatory
        // contract isolated prevents the current task from being parsed as command arguments.
        push_text_block(&mut blocks, required_skills);
    }
    push_text_block(
        &mut blocks,
        render_workspace_boundary(request.worktree_root, request.locale),
    );
    if let Some(role) = render_role_definition(request.node, request.role_content, request.locale) {
        push_text_block(&mut blocks, role);
    }
    push_text_block(
        &mut blocks,
        render_current_step(request.node, request.locale),
    );

    if let Ok(graph) = WorkflowGraph::parse(request.graph_json) {
        push_text_block(
            &mut blocks,
            render_workflow_context(
                &graph,
                request.node,
                request.run_input,
                request.node_runs,
                request.locale,
            ),
        );
    } else if let Some(input) = request.run_input.filter(|input| !input.trim().is_empty()) {
        push_text_block(&mut blocks, render_original_request(input, request.locale));
    }

    if let Some(contract) = render_structured_output_contract(request.node, request.locale) {
        // Keeping the contract last gives the final-response constraint strong recency while the
        // leading skill invocations remain parseable by Agent CLIs.
        push_text_block(&mut blocks, contract);
    }

    blocks
}

/// Appends a text block while preserving an explicit blank line after the previous text block.
fn push_text_block(blocks: &mut Vec<ContentBlock>, text: String) {
    if let Some(ContentBlock::Text(previous)) = blocks.last_mut()
        && !previous.text.ends_with("\n\n")
    {
        // ACP keeps content blocks distinct, but providers may concatenate their text verbatim.
        previous.text.push_str("\n\n");
    }
    blocks.push(text_block(text));
}

/// Renders the authoritative filesystem boundary so adjacent worktrees cannot be mistaken for input.
fn render_workspace_boundary(worktree_root: &Path, locale: WorkflowRunLocale) -> String {
    let worktree_root = render_prompt_path(worktree_root);
    match locale {
        WorkflowRunLocale::ZhCn => format!(
            "<workspace_boundary>\n你必须只在以下工作区根目录内工作：\n{worktree_root}\n\n该目录是本次工作流运行完整且权威的工作空间。\n\n规则：\n1. 只能在该工作区根目录内读取、搜索、创建、修改和删除文件。\n2. 不要检查、枚举或访问其父目录以及相邻的其他 worktree。\n3. 不要使用 `..` 或绝对路径离开该工作区。\n4. 不要跟随解析目标位于该工作区之外的符号链接或目录联接（junction）。\n5. 运行项目命令时，必须将该工作区根目录作为工作目录。\n6. 不要使用其他 worktree 中的文件、Git 状态或 Agent 输出。\n7. 如果任务似乎需要访问该工作区之外的内容，请停止并报告该需求，不要自行访问。\n8. 修改文件前，确认目标路径解析后仍位于该工作区根目录内。\n</workspace_boundary>"
        ),
        WorkflowRunLocale::EnUs => format!(
            "<workspace_boundary>\nYou MUST work exclusively within the following workspace root:\n{worktree_root}\n\nThis directory is the complete and authoritative workspace for this workflow run.\n\nRules:\n1. Read, search, create, modify, and delete files only within this workspace root.\n2. Do not inspect, enumerate, or access the parent directory or sibling worktrees.\n3. Do not use `..` or absolute paths to leave this workspace.\n4. Do not follow symbolic links or junctions whose resolved target is outside this workspace.\n5. Run project commands with this workspace root as the working directory.\n6. Do not use files, Git state, or Agent output from another worktree.\n7. If the task appears to require access outside this workspace, stop and report the requirement instead of accessing it.\n8. Before making changes, verify that the target path resolves inside this workspace root.\n</workspace_boundary>"
        ),
    }
}

/// Renders the task boundary so the Agent knows which workflow step it owns.
fn render_current_step(node: &WorkflowGraphNode, locale: WorkflowRunLocale) -> String {
    let copy = prompt_copy(locale);
    let mut text = format!("<current_workflow_step>\n{}\n", copy.current_step_intro);
    let _ = writeln!(text, "{}: {}", copy.step, node_label(node));
    if !node.description.trim().is_empty() {
        let _ = writeln!(text, "{}:\n{}", copy.description, node.description.trim());
    }
    let prompt = node
        .agent_config
        .as_ref()
        .map(|config| config.prompt.trim())
        .unwrap_or_default();
    if !prompt.is_empty() {
        let _ = writeln!(text, "{}:\n{prompt}", copy.task_instructions);
    }
    text.push_str("</current_workflow_step>");
    text
}

/// Wraps a resolved role in explicit behavioral instructions instead of appending it raw.
fn render_role_definition(
    node: &WorkflowGraphNode,
    role_content: Option<&str>,
    locale: WorkflowRunLocale,
) -> Option<String> {
    let copy = prompt_copy(locale);
    let role = role_content?.trim();
    if role.is_empty() {
        return None;
    }
    let role_name = node
        .agent_config
        .as_ref()
        .and_then(|config| config.role_id.as_deref())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(copy.default_role);
    Some(format!(
        "<system_instructions>\n{}\n{}: {role_name}\n\n{role}\n</system_instructions>",
        copy.role_intro, copy.role
    ))
}

/// Renders topology, live statuses, and the original workflow input.
fn render_workflow_context(
    graph: &WorkflowGraph,
    current_node: &WorkflowGraphNode,
    run_input: Option<&str>,
    node_runs: &[WorkflowNodeRun],
    locale: WorkflowRunLocale,
) -> String {
    let copy = prompt_copy(locale);
    let ordered_nodes = graph.nodes_in_topological_order();
    let order_by_id: HashMap<&str, usize> = ordered_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect();
    let run_by_node_id: HashMap<&str, &WorkflowNodeRun> = node_runs
        .iter()
        .map(|node_run| (node_run.node_id.as_str(), node_run))
        .collect();

    let mut text = format!(
        "<workflow_context>\n## {}\n{}\n",
        copy.workflow_overview, copy.workflow_overview_intro
    );
    let predecessors = sort_nodes_by_topology(graph.predecessors(&current_node.id), &order_by_id);
    let successors = sort_nodes_by_topology(graph.successors(&current_node.id), &order_by_id);
    let _ = writeln!(
        text,
        "{}: {}",
        copy.direct_predecessors,
        node_list(predecessors, locale)
    );
    let _ = writeln!(
        text,
        "{}: {}",
        copy.direct_successors,
        node_list(successors, locale)
    );
    let _ = write!(text, "\n{}:\n", copy.topology_and_status);
    for node in &ordered_nodes {
        let status = run_by_node_id
            .get(node.id.as_str())
            .map(|node_run| node_status_label(node_run.status, locale))
            .unwrap_or(copy.not_started);
        let current = if node.id == current_node.id {
            copy.current_marker
        } else {
            ""
        };
        let successors = sort_nodes_by_topology(graph.successors(&node.id), &order_by_id);
        let _ = writeln!(
            text,
            "- [{status}{current}] {} -> {}",
            node_label(node),
            node_list(successors, locale)
        );
    }

    if let Some(input) = run_input.filter(|input| !input.trim().is_empty()) {
        text.push('\n');
        text.push_str(&render_original_request(input, locale));
        text.push('\n');
    }

    text.push_str("</workflow_context>");
    text
}

/// Renders the run's kickoff request as a distinct workflow-level instruction.
fn render_original_request(input: &str, locale: WorkflowRunLocale) -> String {
    let copy = prompt_copy(locale);
    format!(
        "## {}\n{}\n{}",
        copy.original_request,
        copy.original_request_intro,
        input.trim()
    )
}

/// Renders the JSON contract that constrains only the Agent's final assistant response.
fn render_structured_output_contract(
    node: &WorkflowGraphNode,
    locale: WorkflowRunLocale,
) -> Option<String> {
    let AgentOutputContract::Structured { schema, .. } = node
        .agent_config
        .as_ref()
        .and_then(|config| config.output_contract.as_ref())?
    else {
        return None;
    };
    let example = structured_output_example(schema);
    let example = serde_json::to_string_pretty(&example).unwrap_or_else(|_| example.to_string());
    let schema = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
    Some(match locale {
        WorkflowRunLocale::ZhCn => format!(
            "<structured_output_contract>\n本工作流步骤需要结构化输出。你可以正常思考、调用工具、读写文件并执行任务；以下规则只约束你的最终助手回复。\n\n规则：\n1. 最终助手回复必须只包含一个 JSON 对象。\n2. 不要使用 Markdown 代码块，也不要在 JSON 前后添加解释、标题或其他文字。\n3. Schema 中每个字段的 type 就是变量池要求的类型；必须按类型和下方示例的数据形状输出。\n4. 必须包含 Schema 中声明的所有 required 字段。\n5. 当 additionalProperties 为 false 时，不得添加未声明的字段。\n6. file 类型必须写成 {{\"kind\":\"workspace_file\",\"path\":\"相对于本次运行 Workspace 的路径\"}}；array[file] 是此类对象的数组。不得使用绝对路径或包含 .. 的路径。\n\n合法输出示例：\n{example}\n\nJSON Schema：\n{schema}\n</structured_output_contract>"
        ),
        WorkflowRunLocale::EnUs => format!(
            "<structured_output_contract>\nThis workflow step requires structured output. You may reason, call tools, read or write files, and perform the task normally; the rules below constrain only your final assistant response.\n\nRules:\n1. The final assistant response must contain exactly one JSON object and nothing else.\n2. Do not use a Markdown code fence or add explanations, headings, or other text before or after the JSON.\n3. Each Schema field's type is the type required by the variable pool; follow both the types and the data shapes in the example below.\n4. Include every field declared in the schema's required list.\n5. When additionalProperties is false, do not add undeclared fields.\n6. A file value must be {{\"kind\":\"workspace_file\",\"path\":\"path relative to this run's Workspace\"}}; array[file] is an array of those objects. Never use an absolute path or a path containing `..`.\n\nValid output example:\n{example}\n\nJSON Schema:\n{schema}\n</structured_output_contract>"
        ),
    })
}

/// Builds a valid few-shot value from the same schema vocabulary enforced by the variable pool.
fn structured_output_example(schema: &serde_json::Value) -> serde_json::Value {
    let value_type = schema
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("any");
    match value_type {
        "object" => serde_json::Value::Object(
            schema
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .into_iter()
                .flat_map(|properties| properties.iter())
                .map(|(name, property)| (name.clone(), structured_output_example(property)))
                .collect(),
        ),
        "array" => serde_json::Value::Array(
            schema
                .get("items")
                .map(structured_output_example)
                .into_iter()
                .collect(),
        ),
        "string" => serde_json::json!("text"),
        "number" => serde_json::json!(1.5),
        "integer" => serde_json::json!(1),
        "boolean" => serde_json::json!(true),
        "secret" => serde_json::json!("token-value"),
        "file" => serde_json::json!({
            "kind": "workspace_file",
            "path": "docs/example.txt"
        }),
        "array[string]" => serde_json::json!(["text"]),
        "array[number]" => serde_json::json!([1.5]),
        "array[object]" => serde_json::json!([{}]),
        "array[boolean]" => serde_json::json!([true]),
        "array[file]" => serde_json::json!([{
            "kind": "workspace_file",
            "path": "docs/example.txt"
        }]),
        "array[any]" => serde_json::json!(["text"]),
        "any" => serde_json::json!("text"),
        "null" => serde_json::Value::Null,
        _ => serde_json::Value::Null,
    }
}

/// Renders one node's human title and stable identifier.
fn node_label(node: &WorkflowGraphNode) -> String {
    let title = node.title.trim();
    if title.is_empty() {
        format!("`{}`", node.id)
    } else {
        format!("\"{title}\" (`{}`)", node.id)
    }
}

/// Renders a node list without implying a successor when the edge set is empty.
fn node_list(nodes: Vec<&WorkflowGraphNode>, locale: WorkflowRunLocale) -> String {
    if nodes.is_empty() {
        return prompt_copy(locale).none.to_string();
    }
    nodes
        .iter()
        .map(|node| node_label(node))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Orders one adjacency set by the same stable topology used for the full workflow panorama.
fn sort_nodes_by_topology<'a>(
    mut nodes: Vec<&'a WorkflowGraphNode>,
    order_by_id: &HashMap<&str, usize>,
) -> Vec<&'a WorkflowGraphNode> {
    nodes.sort_by_key(|node| {
        order_by_id
            .get(node.id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    nodes
}

/// Maps persisted node state onto concise workflow-language labels for the Agent.
fn node_status_label(status: WorkflowNodeStatus, locale: WorkflowRunLocale) -> &'static str {
    match (locale, status) {
        (WorkflowRunLocale::ZhCn, WorkflowNodeStatus::Pending) => "等待输入",
        (WorkflowRunLocale::ZhCn, WorkflowNodeStatus::Running) => "运行中",
        (WorkflowRunLocale::ZhCn, WorkflowNodeStatus::Succeeded) => "已成功",
        (WorkflowRunLocale::ZhCn, WorkflowNodeStatus::Failed) => "已失败",
        (WorkflowRunLocale::ZhCn, WorkflowNodeStatus::Cancelled) => "已取消",
        (WorkflowRunLocale::EnUs, WorkflowNodeStatus::Pending) => "awaiting_input",
        (WorkflowRunLocale::EnUs, WorkflowNodeStatus::Running) => "running",
        (WorkflowRunLocale::EnUs, WorkflowNodeStatus::Succeeded) => "succeeded",
        (WorkflowRunLocale::EnUs, WorkflowNodeStatus::Failed) => "failed",
        (WorkflowRunLocale::EnUs, WorkflowNodeStatus::Cancelled) => "cancelled",
    }
}

/// Localized prose injected by Ora around user-authored workflow content.
struct PromptCopy {
    current_step_intro: &'static str,
    step: &'static str,
    description: &'static str,
    task_instructions: &'static str,
    role_intro: &'static str,
    role: &'static str,
    default_role: &'static str,
    workflow_overview: &'static str,
    workflow_overview_intro: &'static str,
    direct_predecessors: &'static str,
    direct_successors: &'static str,
    topology_and_status: &'static str,
    not_started: &'static str,
    current_marker: &'static str,
    none: &'static str,
    original_request: &'static str,
    original_request_intro: &'static str,
}

/// Selects workflow prompt copy from the display language frozen on the run.
fn prompt_copy(locale: WorkflowRunLocale) -> PromptCopy {
    match locale {
        WorkflowRunLocale::ZhCn => PromptCopy {
            current_step_intro: "你负责执行下述工作流步骤。请专注于当前步骤，并让最终回复成为可供后续步骤直接使用的清晰交付内容。",
            step: "当前步骤",
            description: "步骤说明",
            task_instructions: "任务要求",
            role_intro: "在整个步骤中，请遵守以下角色定义，并将其视为对推理、行动和交付表达方式的约束。",
            role: "角色",
            default_role: "工作流角色",
            workflow_overview: "工作流全景",
            workflow_overview_intro: "以下是当前步骤开始时的完整工作流快照。请据此理解自己所处的位置及后续交付方向；除非当前任务明确要求，否则不要代替其他节点执行任务。",
            direct_predecessors: "直接前置节点",
            direct_successors: "直接后续节点",
            topology_and_status: "执行拓扑与当前状态",
            not_started: "未开始",
            current_marker: "，当前节点",
            none: "（无）",
            original_request: "工作流原始请求",
            original_request_intro: "本次工作流由以下请求启动。请将其作为全局目标，同时优先遵守范围更具体的当前步骤要求。",
        },
        WorkflowRunLocale::EnUs => PromptCopy {
            current_step_intro: "You are responsible for the workflow step identified below. Focus on this step and make your final response a clear handoff for downstream steps.",
            step: "Step",
            description: "Description",
            task_instructions: "Task instructions",
            role_intro: "Follow the role definition below throughout this workflow step. Treat it as constraints on how you reason, act, and present the handoff.",
            role: "Role",
            default_role: "workflow role",
            workflow_overview: "Workflow overview",
            workflow_overview_intro: "This is the complete workflow snapshot at the moment this step starts. Use it to understand your position and the downstream handoff; do not perform other nodes' tasks unless your current instructions explicitly require it.",
            direct_predecessors: "Direct predecessors",
            direct_successors: "Direct successors",
            topology_and_status: "Execution topology and current status",
            not_started: "not_started",
            current_marker: ", current",
            none: "(none)",
            original_request: "Original workflow request",
            original_request_intro: "The workflow was started with the following request. Use it as global intent while obeying the narrower current-step instructions.",
        },
    }
}

/// Renders executable slash commands followed by a localized mandatory-skill contract.
fn render_required_skills(skills: &[RequiredWorkflowSkill], locale: WorkflowRunLocale) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let invocation = skills
        .iter()
        .map(|skill| format!("/{}", skill.invocation_name))
        .collect::<Vec<_>>()
        .join(" ");
    let required = skills
        .iter()
        .map(|skill| format!("- /{}", skill.invocation_name))
        .collect::<Vec<_>>()
        .join("\n");
    let locations = skills
        .iter()
        .map(|skill| {
            let paths = skill
                .package_paths
                .iter()
                .map(|path| format!("  - {}", render_prompt_path(path)))
                .collect::<Vec<_>>()
                .join("\n");
            format!("- /{}\n{paths}", skill.invocation_name)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let contract = match locale {
        WorkflowRunLocale::ZhCn => format!(
            "<required_skills>\n\n\
你必须在执行工作流步骤前调用以下技能。\n\n\
必需技能：\n{required}\n\n\
以下技能包已物化到本次工作流的权威工作区：\n{locations}\n\n\
调用技能时，必须使用上述本次运行中的物化副本。\n\n\
技能调用是强制性的前置条件。\n\n\
规则：\n\
1. 在分析任务之前，调用上方列出的每个必需技能。\n\
2. 不要以你自己对技能的理解代替技能调用。\n\
3. 在调用所有必需技能之前，不要开始项目探索、代码阅读、推理或任务执行。\n\
4. 调用后，遵循每项技能返回的说明。\n\
5. 如列出多个技能，必须全部调用后再继续。\n\
6. 如果有任何必需技能尚未调用，不要声称任务已完成。\n\n\
</required_skills>"
        ),
        WorkflowRunLocale::EnUs => format!(
            "<required_skills>\n\n\
You MUST invoke the following skills before performing the workflow step.\n\n\
Required skills:\n{required}\n\n\
The skill packages have been materialized in this workflow run's authoritative workspace:\n{locations}\n\n\
When invoking a skill, you MUST use the materialized copy for this run shown above.\n\n\
Skill invocation is a mandatory prerequisite.\n\n\
Rules:\n\
1. Invoke every required skill listed above before analyzing the task.\n\
2. Do not replace skill invocation with your own interpretation of the skill.\n\
3. Do not begin project exploration, code reading, reasoning, or task execution before the required skills have been invoked.\n\
4. After invocation, follow the instructions returned by each skill.\n\
5. If multiple skills are listed, invoke all of them before proceeding.\n\
6. Do not claim completion if any required skill has not been invoked.\n\n\
</required_skills>"
        ),
    };
    format!("{invocation}\n\n{contract}")
}

/// Renders a filesystem path with portable separators for Agent-facing prompt text.
fn render_prompt_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

/// Wraps owned prompt text as one ACP content block.
fn text_block(text: String) -> ContentBlock {
    ContentBlock::Text(TextContent::new(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_application::{
        AgentConfig, AgentExecutor, AgentOutputContract, NodeType, StructuredTextExposure,
    };
    use ora_domain::{AuditFields, WorkflowNodeRunId, WorkflowRunId};
    use pretty_assertions::assert_eq;

    const GRAPH: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start","title":"Start"}},
            {"id":"research","data":{"kind":"agent","title":"Research","agentConfig":{"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"research"}}},
            {"id":"review","data":{"kind":"agent","title":"Review","description":"Check the evidence.","agentConfig":{"executor":{"agentCli":"open_code","modelId":"m"},"roleId":"Reviewer","prompt":"produce the decision"}}},
            {"id":"output","data":{"kind":"output","title":"Deliver"}}
        ],
        "edges": [
            {"source":"start","target":"research"},
            {"source":"research","target":"review"},
            {"source":"review","target":"output"}
        ]
    }"#;

    /// Builds one persisted node snapshot for prompt-context tests.
    fn node_run(
        node_id: &str,
        status: WorkflowNodeStatus,
        output: Option<&str>,
    ) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new(format!("node-run-{node_id}")),
            WorkflowRunId::new("run-1"),
            node_id,
            "agent",
            /*session_id*/ None,
            status,
            /*input*/ None,
            output.map(str::to_string),
            /*error*/ None,
            /*payload*/ None,
            /*started_at*/ Some(1),
            /*finished_at*/ None,
            AuditFields::new(1, 1, /*is_deleted*/ false),
        )
    }

    /// Extracts text blocks because workflow prompts intentionally contain no binary content.
    fn block_texts(blocks: Vec<ContentBlock>) -> Vec<String> {
        blocks
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn assembly_keeps_skill_invocations_first_and_requires_every_skill() {
        let worktree_root = Path::new("worktrees").join("run-1");
        let node = WorkflowGraphNode {
            id: "review".to_string(),
            node_type: NodeType::Agent,
            title: "Review".to_string(),
            description: "Check the evidence.".to_string(),
            instruction: None,
            input_variables: Vec::new(),
            agent_config: Some(AgentConfig {
                executor: AgentExecutor {
                    agent_cli: "open_code".to_string(),
                    model_id: "m".to_string(),
                },
                role_id: Some("Reviewer".to_string()),
                skills: Vec::new(),
                prompt: "produce the decision".to_string(),
                interactive: false,
                output_contract: None,
            }),
            condition_config: None,
            output_config: None,
        };
        let raw_texts = block_texts(assemble_workflow_prompt(WorkflowPromptRequest {
            node: &node,
            worktree_root: &worktree_root,
            role_content: Some("Be rigorous."),
            graph_json: "invalid",
            run_input: Some("Audit the change."),
            node_runs: &[],
            required_skills: &[
                RequiredWorkflowSkill {
                    invocation_name: "review".to_string(),
                    package_paths: vec![
                        worktree_root.join(".agents").join("skills").join("review"),
                    ],
                },
                RequiredWorkflowSkill {
                    invocation_name: "verify".to_string(),
                    package_paths: vec![
                        worktree_root.join(".agents").join("skills").join("verify"),
                    ],
                },
            ],
            locale: WorkflowRunLocale::EnUs,
        }));
        assert!(
            raw_texts[..raw_texts.len() - 1]
                .iter()
                .all(|text| text.ends_with("\n\n"))
        );
        assert!(!raw_texts.last().unwrap().ends_with("\n\n"));
        assert!(
            raw_texts
                .join("")
                .contains("</required_skills>\n\n<workspace_boundary>")
        );
        let texts = raw_texts
            .into_iter()
            .map(|text| text.strip_suffix("\n\n").unwrap_or(&text).to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            texts,
            vec![
                format!(
                    "/review /verify\n\n<required_skills>\n\nYou MUST invoke the following skills before performing the workflow step.\n\nRequired skills:\n- /review\n- /verify\n\nThe skill packages have been materialized in this workflow run's authoritative workspace:\n- /review\n  - {}\n- /verify\n  - {}\n\nWhen invoking a skill, you MUST use the materialized copy for this run shown above.\n\nSkill invocation is a mandatory prerequisite.\n\nRules:\n1. Invoke every required skill listed above before analyzing the task.\n2. Do not replace skill invocation with your own interpretation of the skill.\n3. Do not begin project exploration, code reading, reasoning, or task execution before the required skills have been invoked.\n4. After invocation, follow the instructions returned by each skill.\n5. If multiple skills are listed, invoke all of them before proceeding.\n6. Do not claim completion if any required skill has not been invoked.\n\n</required_skills>",
                    render_prompt_path(
                        &worktree_root
                            .join(".agents")
                            .join("skills")
                            .join("review")
                    ),
                    render_prompt_path(
                        &worktree_root
                            .join(".agents")
                            .join("skills")
                            .join("verify")
                    )
                ),
                format!(
                    "<workspace_boundary>\nYou MUST work exclusively within the following workspace root:\n{}\n\nThis directory is the complete and authoritative workspace for this workflow run.\n\nRules:\n1. Read, search, create, modify, and delete files only within this workspace root.\n2. Do not inspect, enumerate, or access the parent directory or sibling worktrees.\n3. Do not use `..` or absolute paths to leave this workspace.\n4. Do not follow symbolic links or junctions whose resolved target is outside this workspace.\n5. Run project commands with this workspace root as the working directory.\n6. Do not use files, Git state, or Agent output from another worktree.\n7. If the task appears to require access outside this workspace, stop and report the requirement instead of accessing it.\n8. Before making changes, verify that the target path resolves inside this workspace root.\n</workspace_boundary>",
                    render_prompt_path(&worktree_root)
                ),
                "<system_instructions>\nFollow the role definition below throughout this workflow step. Treat it as constraints on how you reason, act, and present the handoff.\nRole: Reviewer\n\nBe rigorous.\n</system_instructions>".to_string(),
                "<current_workflow_step>\nYou are responsible for the workflow step identified below. Focus on this step and make your final response a clear handoff for downstream steps.\nStep: \"Review\" (`review`)\nDescription:\nCheck the evidence.\nTask instructions:\nproduce the decision\n</current_workflow_step>".to_string(),
                "## Original workflow request\nThe workflow was started with the following request. Use it as global intent while obeying the narrower current-step instructions.\nAudit the change.".to_string(),
            ]
        );
    }

    /// A structured node receives a final-response-only contract after every ordinary context block.
    #[test]
    fn assembly_appends_the_structured_output_contract_last() {
        let node = WorkflowGraphNode {
            id: "review".to_string(),
            node_type: NodeType::Agent,
            title: "Review".to_string(),
            description: String::new(),
            instruction: None,
            input_variables: Vec::new(),
            agent_config: Some(AgentConfig {
                executor: AgentExecutor {
                    agent_cli: "open_code".to_string(),
                    model_id: "m".to_string(),
                },
                role_id: None,
                skills: Vec::new(),
                prompt: "Review the proposal.".to_string(),
                interactive: false,
                output_contract: Some(AgentOutputContract::Structured {
                    schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "approved": { "type": "boolean" },
                            "attachment": { "type": "file" }
                        },
                        "required": ["approved"],
                        "additionalProperties": false
                    }),
                    text_exposure: StructuredTextExposure::StructuredOnly,
                }),
            }),
            condition_config: None,
            output_config: None,
        };
        let texts = block_texts(assemble_workflow_prompt(WorkflowPromptRequest {
            node: &node,
            worktree_root: Path::new("worktrees").join("run-1").as_path(),
            role_content: None,
            graph_json: "invalid",
            run_input: Some("Audit the change."),
            node_runs: &[],
            required_skills: &[],
            locale: WorkflowRunLocale::ZhCn,
        }));

        assert_eq!(texts.len(), 4);
        assert!(texts[2].contains("Audit the change."));
        assert!(texts[3].starts_with("<structured_output_contract>"));
        assert!(texts[3].contains("以下规则只约束你的最终助手回复"));
        assert!(texts[3].contains("合法输出示例"));
        assert!(texts[3].contains(r#""kind": "workspace_file""#));
        assert!(texts[3].contains(r#""approved": true"#));
        assert!(texts[3].contains("\"approved\": {"));
        assert!(texts[3].ends_with("</structured_output_contract>"));
    }

    #[test]
    fn workspace_boundary_uses_chinese_copy_for_a_chinese_run() {
        let worktree_root = Path::new("worktrees").join("run-1");

        assert_eq!(
            render_workspace_boundary(&worktree_root, WorkflowRunLocale::ZhCn),
            format!(
                "<workspace_boundary>\n你必须只在以下工作区根目录内工作：\n{}\n\n该目录是本次工作流运行完整且权威的工作空间。\n\n规则：\n1. 只能在该工作区根目录内读取、搜索、创建、修改和删除文件。\n2. 不要检查、枚举或访问其父目录以及相邻的其他 worktree。\n3. 不要使用 `..` 或绝对路径离开该工作区。\n4. 不要跟随解析目标位于该工作区之外的符号链接或目录联接（junction）。\n5. 运行项目命令时，必须将该工作区根目录作为工作目录。\n6. 不要使用其他 worktree 中的文件、Git 状态或 Agent 输出。\n7. 如果任务似乎需要访问该工作区之外的内容，请停止并报告该需求，不要自行访问。\n8. 修改文件前，确认目标路径解析后仍位于该工作区根目录内。\n</workspace_boundary>",
                render_prompt_path(&worktree_root)
            )
        );
    }

    #[test]
    fn required_skills_use_chinese_copy_for_a_chinese_run() {
        assert_eq!(
            render_required_skills(
                &[RequiredWorkflowSkill {
                    invocation_name: "openspec-explore".to_string(),
                    package_paths: vec![
                        PathBuf::from("worktrees")
                            .join("run-1")
                            .join(".agents")
                            .join("skills")
                            .join("openspec-explore")
                    ],
                }],
                WorkflowRunLocale::ZhCn,
            ),
            format!(
                "/openspec-explore\n\n<required_skills>\n\n你必须在执行工作流步骤前调用以下技能。\n\n必需技能：\n- /openspec-explore\n\n以下技能包已物化到本次工作流的权威工作区：\n- /openspec-explore\n  - {}\n\n调用技能时，必须使用上述本次运行中的物化副本。\n\n技能调用是强制性的前置条件。\n\n规则：\n1. 在分析任务之前，调用上方列出的每个必需技能。\n2. 不要以你自己对技能的理解代替技能调用。\n3. 在调用所有必需技能之前，不要开始项目探索、代码阅读、推理或任务执行。\n4. 调用后，遵循每项技能返回的说明。\n5. 如列出多个技能，必须全部调用后再继续。\n6. 如果有任何必需技能尚未调用，不要声称任务已完成。\n\n</required_skills>",
                render_prompt_path(
                    &PathBuf::from("worktrees")
                        .join("run-1")
                        .join(".agents")
                        .join("skills")
                        .join("openspec-explore")
                )
            )
        );
    }

    #[cfg(windows)]
    #[test]
    fn agent_facing_paths_use_forward_slashes_on_windows() {
        let path = Path::new(r"D:\Projects\desktop")
            .join(".data")
            .join("worktrees")
            .join("run-1")
            .join(".agents")
            .join("skills")
            .join("openspec-explore");

        assert_eq!(
            render_prompt_path(&path),
            "D:/Projects/desktop/.data/worktrees/run-1/.agents/skills/openspec-explore"
        );
    }

    #[test]
    fn workflow_context_names_topology_without_implicit_upstream_handoffs() {
        let graph = WorkflowGraph::parse(GRAPH).unwrap();
        let current = graph.node("review").unwrap();
        let node_runs = vec![
            node_run("start", WorkflowNodeStatus::Succeeded, None),
            node_run(
                "research",
                WorkflowNodeStatus::Succeeded,
                Some("final evidence"),
            ),
            node_run("review", WorkflowNodeStatus::Running, None),
        ];
        let context = render_workflow_context(
            &graph,
            current,
            Some("Audit the change."),
            &node_runs,
            WorkflowRunLocale::EnUs,
        );
        let topology: Vec<_> = context
            .lines()
            .filter(|line| line.starts_with("- ["))
            .collect();
        assert_eq!(
            topology,
            vec![
                "- [succeeded] \"Start\" (`start`) -> \"Research\" (`research`)",
                "- [succeeded] \"Research\" (`research`) -> \"Review\" (`review`)",
                "- [running, current] \"Review\" (`review`) -> \"Deliver\" (`output`)",
                "- [not_started] \"Deliver\" (`output`) -> (none)",
            ]
        );
        assert_eq!(context.contains("final evidence"), false);
        assert_eq!(context.contains("Available upstream node outputs"), false);
        assert_eq!(
            context.contains("## Original workflow request\nThe workflow was started with"),
            true
        );
        assert_eq!(
            context.contains("Direct predecessors: \"Research\" (`research`)"),
            true
        );
        assert_eq!(
            context.contains("Current step: \"Review\" (`review`)"),
            false
        );
        assert_eq!(
            context.contains("Direct successors: \"Deliver\" (`output`)"),
            true
        );
    }

    #[test]
    fn assembly_uses_chinese_copy_for_a_chinese_run() {
        let graph = WorkflowGraph::parse(GRAPH).unwrap();
        let current = graph.node("review").unwrap();
        let context = render_workflow_context(
            &graph,
            current,
            Some("审查这次修改。"),
            &[node_run("review", WorkflowNodeStatus::Running, None)],
            WorkflowRunLocale::ZhCn,
        );

        assert_eq!(
            context.contains("## 工作流全景\n以下是当前步骤开始时的完整工作流快照。"),
            true
        );
        assert_eq!(context.contains("[运行中，当前节点]"), true);
        assert_eq!(context.contains("## 工作流原始请求"), true);
        assert_eq!(context.contains("Direct successors"), false);
    }

    /// Node-run outputs never enter another node's context without an explicit variable token.
    #[test]
    fn workflow_context_excludes_all_node_outputs() {
        let graph = WorkflowGraph::parse(GRAPH).unwrap();
        let current = graph.node("review").unwrap();
        let node_runs = vec![
            node_run("start", WorkflowNodeStatus::Succeeded, Some("kickoff")),
            node_run(
                "research",
                WorkflowNodeStatus::Succeeded,
                Some("final evidence"),
            ),
            node_run("review", WorkflowNodeStatus::Running, None),
        ];
        let context = render_workflow_context(
            &graph,
            current,
            Some("kickoff"),
            &node_runs,
            WorkflowRunLocale::EnUs,
        );
        assert_eq!(context.contains("final evidence"), false);
    }

    /// Missing predecessor output does not change the explicit-only context shape.
    #[test]
    fn workflow_context_excludes_a_succeeded_predecessor_with_no_output() {
        let graph = WorkflowGraph::parse(GRAPH).unwrap();
        let current = graph.node("review").unwrap();
        let node_runs = vec![
            node_run("start", WorkflowNodeStatus::Succeeded, None),
            // The research node succeeded but produced no output (output policy = none).
            node_run("research", WorkflowNodeStatus::Succeeded, None),
            node_run("review", WorkflowNodeStatus::Running, None),
        ];
        let context = render_workflow_context(
            &graph,
            current,
            Some("kickoff"),
            &node_runs,
            WorkflowRunLocale::EnUs,
        );
        let upstream: Vec<_> = context
            .lines()
            .filter(|line| line.starts_with("### "))
            .collect();
        assert_eq!(upstream, Vec::<&str>::new());
    }
}
