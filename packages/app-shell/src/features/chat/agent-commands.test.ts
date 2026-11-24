import { describe, expect, it } from "vitest";
import { discoveredCommand, presentableCommands } from "./agent-commands";

/** Builds a command whose other fields do not matter to the case. */
function named(name: string) {
  return { name, description: "" };
}

/** Counts the bytes a limit is expressed in. */
function bytes(text: string) {
  return new TextEncoder().encode(text).length;
}

describe("presentable commands", () => {
  it("keeps the agent's order and stops after 500", () => {
    const names = Array.from({ length: 501 }, (_, index) => `c${index}`);

    expect(
      presentableCommands(names.map(named)).map(({ name }) => name),
    ).toEqual(names.slice(0, 500));
  });

  it("drops an unusable name alone", () => {
    const kept = [
      "developer-test-agent:generate-dt-files",
      "a_b.c",
      "命令",
      "x".repeat(128),
    ];
    const dropped = [
      "",
      "two words",
      "tab\tname",
      "line\nname",
      "bell\u0007",
      "c1\u0085",
      "x".repeat(129),
      "字".repeat(43),
    ];
    const mixed = kept.flatMap((name, index) => [
      name,
      ...dropped.slice(index * 2, index * 2 + 2),
    ]);

    expect(
      presentableCommands(mixed.map(named)).map(({ name }) => name),
    ).toEqual(kept);
  });

  it("cuts descriptions and hints on a character boundary", () => {
    const [command] = presentableCommands([
      {
        name: "long",
        // 4096 and 256 both fall inside a three-byte character.
        description: "字".repeat(2000),
        input: { hint: "字".repeat(100) },
      },
    ]);

    expect(command).toEqual({
      name: "long",
      description: `${"字".repeat(1365)}…`,
      input: { hint: "字".repeat(85) },
    });
  });

  it("leaves text at the limit alone, including characters outside the BMP", () => {
    const command = {
      name: "fits",
      description: "😀".repeat(1024),
      input: { hint: "x".repeat(256) },
    };
    expect(bytes(command.description)).toBe(4096);

    expect(presentableCommands([command])).toEqual([command]);
    expect(
      presentableCommands([{ ...command, description: "😀".repeat(1025) }])[0]
        ?.description,
    ).toBe(`${"😀".repeat(1024)}…`);
  });
});

describe("discovered commands", () => {
  it("take the shape a session reports", () => {
    expect(
      discoveredCommand({
        name: "review",
        description: "Review",
        inputHint: "scope",
      }),
    ).toEqual({
      name: "review",
      description: "Review",
      input: { hint: "scope" },
    });
    expect(discoveredCommand({ name: "init", description: "Set up" })).toEqual({
      name: "init",
      description: "Set up",
    });
  });
});
