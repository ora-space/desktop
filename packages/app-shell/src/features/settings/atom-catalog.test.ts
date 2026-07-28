import { describe, expect, it } from "vitest";
import {
  CATALOG_AGENTS,
  CATALOG_SKILLS,
  COMMON_AGENT_IDS,
  COMMON_SKILL_IDS,
  isCatalogAtom,
  isInternalAtom,
} from "./atom-catalog";

describe("atom catalog", () => {
  it("keeps intentionally uneven, unique role and skill catalogs", () => {
    expect(CATALOG_AGENTS).toHaveLength(57);
    expect(CATALOG_SKILLS).toHaveLength(63);
    expect(new Set(CATALOG_AGENTS.map(({ id }) => id)).size).toBe(CATALOG_AGENTS.length);
    expect(new Set(CATALOG_SKILLS.map(({ id }) => id)).size).toBe(CATALOG_SKILLS.length);
    expect(CATALOG_AGENTS.every(isCatalogAtom)).toBe(true);
    expect(CATALOG_SKILLS.every(isCatalogAtom)).toBe(true);
  });

  it("puts cdase-build first and raises storage-oriented internal work without grouping it rigidly", () => {
    expect(CATALOG_SKILLS[0]?.name).toBe("cdase-build");
    expect(CATALOG_AGENTS.slice(0, 12).filter(isInternalAtom)).toHaveLength(2);
    expect(CATALOG_SKILLS.slice(0, 16).filter(isInternalAtom)).toHaveLength(2);
  });

  it("references valid records from both common sections", () => {
    expect(COMMON_AGENT_IDS.every((id) => CATALOG_AGENTS.some((item) => item.id === id))).toBe(true);
    expect(COMMON_SKILL_IDS.every((id) => CATALOG_SKILLS.some((item) => item.id === id))).toBe(true);
  });
});
