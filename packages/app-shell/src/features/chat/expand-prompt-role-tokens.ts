import type { Agent, AgentDetails } from "@ora/contracts";

/** One `@role` mention as the composer emits it in a sent prompt's plain text. */
const ROLE_TOKEN_PATTERN = /(?<![A-Za-z0-9/])@([A-Za-z][\w-]*)/g;

/**
 * Expands every `@role` mention in a sent prompt into a context block carrying
 * the role's name, description, and persona content.
 *
 * The transcript keeps the token (`@designer`) so the chip stays readable for
 * the user, but the agent only sees that literal token unless this expansion is
 * appended to `agentText`. Unknown `@names` are left untouched so ordinary text
 * such as a handle or path never changes the prompt.
 *
 * @returns The prompt suffix to attach, or `null` when no known role was referenced.
 */
export async function expandPromptRoleTokens(
  text: string,
  agents: readonly Agent[],
  resolveContent: (agentId: string) => Promise<AgentDetails | undefined>,
): Promise<string | null> {
  const referenced = referencedRoleNames(text, agents);
  if (referenced.length === 0) return null;

  const blocks: string[] = [];
  for (const name of referenced) {
    const agent = agents.find((candidate) => candidate.name === name);
    const details =
      agent === undefined ? undefined : await resolveContent(agent.id);
    const content = details?.content?.trim();
    blocks.push(
      [
        `【角色：${agent!.name}】`,
        `描述：${agent!.description}`,
        ...(content ? [`内容：\n${content}`] : []),
      ].join("\n"),
    );
  }

  return [
    "以下是用户在本次对话中引用的角色完整信息：",
    "",
    blocks.join("\n\n"),
  ].join("\n");
}

/** Role names referenced by `@name` that actually exist in the agent list, in mention order. */
function referencedRoleNames(text: string, agents: readonly Agent[]): string[] {
  const known = new Set(agents.map((agent) => agent.name));
  const seen = new Set<string>();
  const names: string[] = [];
  ROLE_TOKEN_PATTERN.lastIndex = 0;
  for (const match of text.matchAll(ROLE_TOKEN_PATTERN)) {
    const name = match[1] ?? "";
    if (!known.has(name) || seen.has(name)) continue;
    seen.add(name);
    names.push(name);
  }
  return names;
}
