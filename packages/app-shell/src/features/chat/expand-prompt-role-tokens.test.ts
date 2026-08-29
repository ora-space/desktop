import { describe, expect, it } from "vitest";
import { expandPromptRoleTokens } from "./expand-prompt-role-tokens";
import type { Agent, AgentDetails } from "@ora/contracts";

const AGENTS: Agent[] = [
  {
    id: "designer-1",
    namespace: "ora",
    name: "designer",
    description: "Design roles",
  },
];

const DETAILS: AgentDetails = {
  id: "designer-1",
  namespace: "ora",
  name: "designer",
  description: "Design roles",
  content: "You are a senior product designer.",
};

const details = async (agentId: string): Promise<AgentDetails | undefined> =>
  agentId === "designer-1" ? DETAILS : undefined;

describe("expandPromptRoleTokens", () => {
  it("appends the referenced role's name, description, and content", async () => {
    const expansion = await expandPromptRoleTokens(
      "请 @designer 写方案",
      AGENTS,
      details,
    );

    expect(expansion).not.toBeNull();
    expect(expansion).toContain("【角色：designer】");
    expect(expansion).toContain("描述：Design roles");
    expect(expansion).toContain("内容：");
    expect(expansion).toContain("You are a senior product designer.");
  });

  it("returns null when no known role is referenced", async () => {
    const expansion = await expandPromptRoleTokens(
      "hello @stranger world",
      AGENTS,
      details,
    );
    expect(expansion).toBeNull();
  });

  it("does not treat an email handle as a role mention", async () => {
    const expansion = await expandPromptRoleTokens(
      "email user@example.com",
      AGENTS,
      details,
    );
    expect(expansion).toBeNull();
  });

  it("skips a role whose content is absent", async () => {
    const expansion = await expandPromptRoleTokens(
      "请 @designer 写方案",
      AGENTS,
      async () => undefined,
    );
    expect(expansion).not.toBeNull();
    expect(expansion).toContain("【角色：designer】");
    expect(expansion).toContain("描述：Design roles");
    expect(expansion).not.toContain("内容：");
  });
});
