import { describe, expect, it } from "vitest";
import { queryStateFromText } from "./composer-query";

describe("queryStateFromText", () => {
  it("treats / after existing text as a slash command trigger", () => {
    expect(queryStateFromText("hello /re", "hello /re")).toEqual({
      isBlank: false,
      slashQuery: "re",
      atQuery: null,
      atTriggerIndex: null,
    });
  });

  it("treats @ after existing text as a mention trigger", () => {
    expect(queryStateFromText("check @Doc", "check @Doc")).toEqual({
      isBlank: false,
      slashQuery: null,
      atQuery: "Doc",
      atTriggerIndex: 6,
    });
  });

  it("does not treat a mid-word slash as a command", () => {
    expect(queryStateFromText("foo/bar", "foo/bar").slashQuery).toBeNull();
  });

  it("treats a slash after ordinary prompt text as an inline variable trigger", () => {
    expect(
      queryStateFromText("完整方案。/", "完整方案。/", "inline").slashQuery,
    ).toBe("");
  });
});
