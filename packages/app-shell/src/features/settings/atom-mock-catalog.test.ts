import { describe, expect, it } from "vitest";
import {
  MOCK_AGENTS,
  MOCK_SKILLS,
  isHuaweiAtom,
  isMockAtom,
} from "./atom-mock-catalog";

describe("atom mock catalog", () => {
  it("provides 60 unique records for each settings catalog", () => {
    expect(MOCK_AGENTS).toHaveLength(60);
    expect(MOCK_SKILLS).toHaveLength(60);
    expect(new Set(MOCK_AGENTS.map(({ id }) => id)).size).toBe(60);
    expect(new Set(MOCK_SKILLS.map(({ id }) => id)).size).toBe(60);
  });

  it("keeps Huawei-flavoured examples a small, explicitly identifiable subset", () => {
    expect(MOCK_AGENTS.every(isMockAtom)).toBe(true);
    expect(MOCK_SKILLS.every(isMockAtom)).toBe(true);
    expect(MOCK_AGENTS.filter(isHuaweiAtom)).toHaveLength(7);
    expect(MOCK_SKILLS.filter(isHuaweiAtom)).toHaveLength(8);
  });
});
