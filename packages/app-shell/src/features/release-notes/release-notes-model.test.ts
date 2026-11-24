import { describe, expect, it } from "vitest";
import type { ReleaseNote } from "../../lib/release-note";
import { decodeReleaseNotes } from "./bundled-release-notes";
import {
  planReleaseNotesPrompt,
  stableVersionOf,
  visibleReleaseNotes,
} from "./release-notes-model";

/** Builds one version's notes with distinguishable texts. */
function note(version: string): ReleaseNote {
  return {
    version,
    date: "2026-12-10",
    entries: [
      { category: "fix", "zh-CN": `修复 ${version}`, "en-US": version },
    ],
  };
}

const notes = ["0.7.0", "0.10.0", "0.8.1", "0.8.0"].map(note);
const versions = (shown: readonly ReleaseNote[] | undefined) =>
  shown?.map(({ version }) => version);

describe("stableVersionOf", () => {
  it.each([
    ["0.7.0", "0.7.0"],
    ["0.7.0-rc.1", "0.7.0"],
    ["0.7.0+yellow", "0.7.0"],
    ["nightly", null],
  ])("maps %s to %s", (version, expected) => {
    expect(stableVersionOf(version)).toBe(expected);
  });
});

describe("visibleReleaseNotes", () => {
  it("orders versions numerically and never shows a newer one", () => {
    expect(versions(visibleReleaseNotes(notes, "0.10.0"))).toEqual([
      "0.10.0",
      "0.8.1",
      "0.8.0",
      "0.7.0",
    ]);
    expect(versions(visibleReleaseNotes(notes, "0.8.0-rc.2"))).toEqual([
      "0.8.0",
      "0.7.0",
    ]);
  });

  it("shows nothing for a build older than every note", () => {
    expect(visibleReleaseNotes(notes, "0.1.0")).toEqual([]);
  });
});

describe("planReleaseNotesPrompt", () => {
  const plan = (
    appVersion: string,
    promptedVersion: string | null,
    newInstallation = false,
  ) =>
    planReleaseNotesPrompt({
      notes,
      appVersion,
      promptedVersion,
      newInstallation,
    });

  it("lists every version after the record up to the running one", () => {
    const planned = plan("0.10.0", "0.7.0");
    expect(planned?.version).toBe("0.10.0");
    expect(versions(planned?.notes)).toEqual(["0.10.0", "0.8.1", "0.8.0"]);
  });

  it.each([
    ["the same version", "0.8.0", "0.8.0"],
    ["a downgrade", "0.8.0", "0.10.0"],
    ["a stable version after its prerelease", "0.8.0", "0.8.0"],
  ])("does nothing for %s", (_label, appVersion, promptedVersion) => {
    expect(plan(appVersion, promptedVersion)).toBeNull();
  });

  it("remembers a prerelease as its stable version", () => {
    expect(plan("0.8.0-rc.1", "0.7.0")?.version).toBe("0.8.0");
  });

  it.each([null, "0.7.0"])(
    "neither prompts nor remembers a build without its own notes (record %s)",
    (record) => {
      expect(plan("0.9.0", record)).toBeNull();
      expect(plan("0.1.0", record)).toBeNull();
    },
  );

  it("remembers a fresh installation without prompting", () => {
    expect(plan("0.8.0", null, /*newInstallation*/ true)).toEqual({
      version: "0.8.0",
      notes: [],
    });
  });

  it("lists every applicable version after an upgrade without a record", () => {
    expect(versions(plan("0.8.1", null)?.notes)).toEqual([
      "0.8.1",
      "0.8.0",
      "0.7.0",
    ]);
  });
});

describe("decodeReleaseNotes", () => {
  it("keeps well-formed notes", () => {
    expect(decodeReleaseNotes(notes)).toEqual(notes);
  });

  it.each([
    ["nothing", undefined],
    ["a string", "[]"],
    ["an entry without a translation", [{ ...note("0.7.0"), entries: [{}] }]],
    ["one malformed version", [note("0.7.0"), { version: "0.8.0" }]],
  ])("hides all notes when the injection is %s", (_label, value) => {
    expect(decodeReleaseNotes(value)).toEqual([]);
  });
});
