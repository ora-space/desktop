import sharedCases from "../../../../../crates/backend/src/workflow/run/prerequisites/skill-reference-cases.json";
import { describe, expect, it } from "vitest";
import type { Skill } from "@ora/contracts";
import {
  resolveWorkflowSkill,
  workflowSkillInstallQuery,
} from "./workflow-skill-reference";

const fixtures = sharedCases as {
  skills: Omit<Skill, "source" | "description">[];
  cases: { reference: string; status: string; catalogId?: string }[];
};
const skills: Skill[] = fixtures.skills.map((skill) => ({
  ...skill,
  description: "",
  source:
    skill.namespace === "local"
      ? { kind: "local" }
      : { kind: "plugin", pluginId: skill.namespace },
}));

describe("workflow Skill references (shared with launch preflight)", () => {
  it.each(fixtures.cases)(
    "$reference resolves as $status",
    ({ reference, status, catalogId }) => {
      const resolved = resolveWorkflowSkill(reference, skills);
      expect(resolved.status).toBe(status);
      if (catalogId)
        expect("skill" in resolved && resolved.skill.id).toBe(catalogId);
    },
  );
  it("does not fall back through a soft-deleted catalog identity", () => {
    expect(
      resolveWorkflowSkill(
        "retired-id",
        [{ ...skills[0]!, id: "other-id", name: "retired-id" }],
        ["retired-id"],
      ),
    ).toEqual({ status: "missing" });
  });
  it("extracts the plugin identity for marketplace recovery", () => {
    expect(
      workflowSkillInstallQuery("plugin:official/review-pack:review"),
    ).toBe("official/review-pack");
    expect(workflowSkillInstallQuery("official/review-pack")).toBe(
      "official/review-pack",
    );
  });
});
