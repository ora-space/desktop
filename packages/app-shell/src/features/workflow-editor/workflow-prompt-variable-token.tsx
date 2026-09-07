import { NodeViewWrapper, type NodeViewProps } from "@tiptap/react";
import type { PromptTokenKind } from "@ora/editor/composer";
import type { WorkflowNodeKind } from "@ora/workflow-mock";
import { WorkflowVariableDisplay } from "./workflow-variable-display";

/** JSON payload stored on a variable token so its NodeView can render Dify-style. */
interface VariableTokenMeta {
  kind?: WorkflowNodeKind;
  node: string;
  variableName: string;
}

function tokenText(kind: PromptTokenKind, name: string): string {
  if (kind === "variable") return `{{#${name}#}}`;
  if (kind === "command") return `/${name}`;
  if (kind === "role") return `@${name}`;
  return `$${name}`;
}

/** Renders a prompt token: rich Dify-style label for variables, plain text otherwise. */
export function PromptTokenNodeView({ node }: NodeViewProps) {
  const kind = node.attrs.kind as PromptTokenKind;
  const name = String(node.attrs.name);
  const label = String(node.attrs.label);

  if (kind !== "variable") {
    return (
      <NodeViewWrapper
        as="span"
        data-prompt-token={kind}
        className="composer-mention"
        contentEditable={false}
      >
        {label !== "" ? label : tokenText(kind, name)}
      </NodeViewWrapper>
    );
  }

  let meta: VariableTokenMeta | null = null;
  const raw = String(node.attrs.meta ?? "");
  if (raw !== "") {
    try {
      meta = JSON.parse(raw) as VariableTokenMeta;
    } catch {
      meta = null;
    }
  }
  if (meta === null) {
    return (
      <NodeViewWrapper
        as="span"
        data-prompt-token="variable"
        className="composer-mention"
        contentEditable={false}
      >
        {label !== "" ? label : tokenText(kind, name)}
      </NodeViewWrapper>
    );
  }
  const variable = {
    variableName: meta.variableName,
    sourceNodeKind: meta.kind,
  };
  return (
    <NodeViewWrapper
      as="span"
      data-prompt-token="variable"
      data-workflow-prompt-variable=""
      className="composer-mention"
      contentEditable={false}
    >
      <WorkflowVariableDisplay variable={variable} nodeName={meta.node} />
    </NodeViewWrapper>
  );
}
