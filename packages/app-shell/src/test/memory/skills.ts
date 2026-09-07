import { type Skill } from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import { nextId } from "./records";

/** Mutable records owned by the skills test adapter. */
export interface SkillMemoryState {
  skills: Skill[];
}

/** Creates an independent skills memory fixture. */
export function createSkillMemory(): SkillMemoryState {
  return { skills: [] };
}

/** Registers only the skills operations explicitly requested by a fixture. */
export function skillHandlers(state: SkillMemoryState) {
  return {
    listSkills: async () => ({ skills: [...state.skills] }),
    getSkill: async (req) => ({
      skill: {
        ...state.skills.find((s) => s.id === req.skillId)!,
        content: "",
      },
    }),
    createSkill: async (req) => {
      const skill: Skill = {
        id: nextId("sk", state.skills.length),
        namespace: "local",
        name: req.name,
        description: req.description,
        source: { kind: "local" } as const,
        availability: "available",
      };
      state.skills.push(skill);
      return { skill };
    },
    updateSkill: async (req) => {
      const idx = state.skills.findIndex((s) => s.id === req.skillId);
      if (idx < 0) throw new Error(`skill ${req.skillId} not found`);
      const existing = state.skills[idx]!;
      const updated: Skill = {
        id: req.skillId,
        namespace: existing.namespace,
        name: req.name,
        description: req.description,
        source: { kind: "local" } as const,
        availability: existing.availability,
      };
      state.skills[idx] = updated;
      return { skill: updated };
    },
    deleteSkill: async (req) => {
      const idx = state.skills.findIndex((s) => s.id === req.skillId);
      if (idx >= 0) state.skills.splice(idx, 1);
      return { skillId: req.skillId };
    },
    cancelSkillImport: async (request) => ({
      sessionId: request.sessionId,
      cancelled: true,
    }),
  } satisfies TestHandlers;
}
