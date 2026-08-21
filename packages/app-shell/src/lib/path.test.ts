import { describe, expect, it } from "vitest";
import { parentDirectory } from "./path";

describe("parentDirectory", () => {
  it("strips the file name for posix and windows paths", () => {
    expect(parentDirectory("/home/ora/downloads/skill.zip")).toBe(
      "/home/ora/downloads",
    );
    expect(parentDirectory("C:\\Users\\ora\\skill.zip")).toBe("C:\\Users\\ora");
  });

  it("keeps paths that have no usable parent", () => {
    expect(parentDirectory("skill.zip")).toBe("skill.zip");
    expect(parentDirectory("/skill.zip")).toBe("/skill.zip");
  });
});
