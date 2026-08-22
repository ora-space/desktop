import { describe, expect, it } from "vitest";
import { groupByStable } from "./group-by-stable";

describe("groupByStable", () => {
  it("reuses bucket arrays when membership and item identities are unchanged", () => {
    const a = { id: "a", g: "1" };
    const b = { id: "b", g: "1" };
    const c = { id: "c", g: "2" };
    const first = groupByStable([a, b, c], (item) => item.g);
    const second = groupByStable([a, b, c], (item) => item.g, first);

    expect(second.get("1")).toBe(first.get("1"));
    expect(second.get("2")).toBe(first.get("2"));
  });

  it("rebuilds only the bucket whose item identity changed", () => {
    const a = { id: "a", g: "1", title: "old" };
    const b = { id: "b", g: "2" };
    const first = groupByStable([a, b], (item) => item.g);
    const a2 = { ...a, title: "new" };
    const second = groupByStable([a2, b], (item) => item.g, first);

    expect(second.get("1")).not.toBe(first.get("1"));
    expect(second.get("2")).toBe(first.get("2"));
  });
});
