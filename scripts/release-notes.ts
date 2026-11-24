// Release notes are handwritten per stable version in release-notes/<version>.json. This module owns
// their format, and the checks that decide whether a versioned package may ship them.
import { execFile } from "node:child_process";
import { lstat, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";

export const RELEASE_NOTES_DIRECTORY = "release-notes";
export const RELEASE_NOTE_CATEGORIES = [
  "important",
  "feature",
  "improvement",
  "fix",
] as const;
export const RELEASE_NOTE_LOCALES = ["zh-CN", "en-US"] as const;
/** Bounds each localized text, in Unicode code points, so one entry stays readable in a card. */
export const MAX_RELEASE_NOTE_TEXT_LENGTH = 500;
/** Bounds one version, because clients would hide the notes of a longer one. */
export const MAX_RELEASE_NOTE_ENTRIES = 50;

export type ReleaseNoteEntry = {
  category: (typeof RELEASE_NOTE_CATEGORIES)[number];
} & Record<(typeof RELEASE_NOTE_LOCALES)[number], string>;

export interface ReleaseNote {
  schema: 1;
  version: string;
  date: string;
  entries: ReleaseNoteEntry[];
}

const stableVersionPattern = /^(?:0|[1-9]\d*)(?:\.(?:0|[1-9]\d*)){2}$/;
// Texts stay on one line, and bidirectional controls cannot make them read differently from their
// content.
const forbiddenTextPattern = /[\p{Cc}\p{Bidi_Control}\u2028\u2029]/u;

/** Returns the stable version whose notes a build ships, so prereleases and build suffixes share them. */
export function stableVersionOf(version: string): string {
  return version.replace(/[-+].*$/, "");
}

/** Parses one notes file, or throws an error that names the file and the first violated rule. */
export function parseReleaseNote(fileName: string, text: string): ReleaseNote {
  const reject = (reason: string): never => {
    throw new Error(`${RELEASE_NOTES_DIRECTORY}/${fileName}: ${reason}`);
  };
  /** Rejects missing and unknown fields alike, because notes must not carry source PRs or ranges. */
  const requireFields = (
    value: unknown,
    fields: readonly string[],
    label: string,
  ): Record<string, unknown> => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return reject(`${label} must be an object`);
    }
    const actual = Object.keys(value);
    const missing = fields.filter((field) => !actual.includes(field));
    const unknown = actual.filter((field) => !fields.includes(field));
    if (missing.length > 0) reject(`${label} is missing ${missing.join(", ")}`);
    if (unknown.length > 0) {
      reject(`${label} has unknown fields: ${unknown.join(", ")}`);
    }
    return value as Record<string, unknown>;
  };

  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch (error) {
    return reject(`invalid JSON (${(error as Error).message})`);
  }
  const { schema, version, date, entries } = requireFields(
    json,
    ["schema", "version", "date", "entries"],
    "the document",
  );
  if (schema !== 1) return reject("schema must be 1");
  if (typeof version !== "string" || !stableVersionPattern.test(version)) {
    return reject("version must be a stable version such as 0.7.0");
  }
  // Matching names also make versions unique, since one directory cannot repeat a file name.
  if (fileName !== `${version}.json`) {
    return reject(`the file name must be ${version}.json`);
  }
  if (typeof date !== "string" || !isCalendarDate(date)) {
    return reject("date must be a calendar date such as 2026-12-10");
  }
  if (!Array.isArray(entries) || entries.length === 0) {
    return reject("entries must be a non-empty array");
  }
  if (entries.length > MAX_RELEASE_NOTE_ENTRIES) {
    return reject(`entries exceeds ${MAX_RELEASE_NOTE_ENTRIES} items`);
  }
  return {
    schema,
    version,
    date,
    entries: entries.map((entry, index) => {
      const label = `entries[${index}]`;
      const fields = requireFields(
        entry,
        ["category", ...RELEASE_NOTE_LOCALES],
        label,
      );
      const category = RELEASE_NOTE_CATEGORIES.find(
        (candidate) => candidate === fields.category,
      );
      if (category === undefined) {
        return reject(
          `${label}.category must be one of ${RELEASE_NOTE_CATEGORIES.join(", ")}`,
        );
      }
      for (const locale of RELEASE_NOTE_LOCALES) {
        const value = fields[locale];
        if (typeof value !== "string" || value.trim() === "") {
          return reject(`${label}.${locale} must be non-empty text`);
        }
        if ([...value].length > MAX_RELEASE_NOTE_TEXT_LENGTH) {
          return reject(
            `${label}.${locale} exceeds ${MAX_RELEASE_NOTE_TEXT_LENGTH} characters`,
          );
        }
        if (forbiddenTextPattern.test(value)) {
          return reject(
            `${label}.${locale} contains a control, line separator, or bidirectional formatting character`,
          );
        }
      }
      return {
        category,
        "zh-CN": fields["zh-CN"] as string,
        "en-US": fields["en-US"] as string,
      };
    }),
  };
}

/** Accepts only dates that exist, because Date would otherwise roll 2026-02-30 into March. */
function isCalendarDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const parsed = new Date(`${value}T00:00:00Z`);
  return (
    !Number.isNaN(parsed.getTime()) &&
    parsed.toISOString().slice(0, 10) === value
  );
}

/** Reads and validates every notes file that a package would ship; no directory means no notes. */
export async function readReleaseNotes(root: string): Promise<ReleaseNote[]> {
  const directory = path.join(root, RELEASE_NOTES_DIRECTORY);
  const status = await lstat(directory).catch((error) => {
    if (error.code === "ENOENT") return undefined;
    throw error;
  });
  if (status === undefined) return [];
  // Git reports a tracked link as clean whatever it points at, so linked content is not committed
  // content.
  if (!status.isDirectory()) {
    throw new Error(`${RELEASE_NOTES_DIRECTORY} must be a directory`);
  }
  const notes: ReleaseNote[] = [];
  const files = (await readdir(directory, { withFileTypes: true }))
    .filter((file) => file.name.toLowerCase().endsWith(".json"))
    .sort((left, right) => (left.name < right.name ? -1 : 1));
  for (const file of files) {
    if (!file.isFile()) {
      throw new Error(
        `${RELEASE_NOTES_DIRECTORY}/${file.name}: must be a regular file`,
      );
    }
    const bytes = await readFile(path.join(directory, file.name));
    let text: string;
    try {
      // Lenient decoding would turn damaged bytes into replacement characters that still parse.
      text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
        bytes,
      );
    } catch (cause) {
      throw new Error(
        `${RELEASE_NOTES_DIRECTORY}/${file.name}: must be valid UTF-8`,
        { cause },
      );
    }
    notes.push(parseReleaseNote(file.name, text));
  }
  return notes;
}

/** Clients hide every note beyond these limits, so they may never exceed a released client's. */
export const MAX_MANIFEST_VERSIONS = 10;
export const MANIFEST_NOTES_SCHEMA = 1;

/** The notes an update manifest carries, with their own schema because older clients read them. */
export interface ManifestNotes {
  schema: typeof MANIFEST_NOTES_SCHEMA;
  versions: Omit<ReleaseNote, "schema">[];
}

/**
 * Selects the latest notes not above the released stable version. Throws when that version has none or
 * any file is invalid: packaging accepted the same commit, so only a defect can get here, and a
 * manifest must not be published silently without its notes.
 */
export async function selectManifestNotes(
  root: string,
  version: string,
): Promise<ManifestNotes> {
  const released = stableVersionOf(version);
  const notes = (await readReleaseNotes(root)).filter(
    (note) => compareStableVersions(note.version, released) <= 0,
  );
  requireNotesOf(notes, version);
  return {
    schema: MANIFEST_NOTES_SCHEMA,
    // Only fields that clients know travel, so a later file field cannot leak into the manifest.
    versions: notes
      .sort((left, right) => compareStableVersions(right.version, left.version))
      .slice(0, MAX_MANIFEST_VERSIONS)
      .map(({ version, date, entries }) => ({
        version,
        date,
        entries: entries.map((entry) => ({
          category: entry.category,
          "zh-CN": entry["zh-CN"],
          "en-US": entry["en-US"],
        })),
      })),
  };
}

/** Throws unless the notes include the stable version that a release or package ships. */
function requireNotesOf(notes: ReleaseNote[], version: string): void {
  const stableVersion = stableVersionOf(version);
  if (!notes.some((note) => note.version === stableVersion)) {
    throw new Error(
      `Packaging ${version} requires ${RELEASE_NOTES_DIRECTORY}/${stableVersion}.json`,
    );
  }
}

/** Orders stable versions by their numbers, since text order would put 0.10.0 before 0.9.0. */
function compareStableVersions(left: string, right: string): number {
  const rightParts = right.split(".").map(Number);
  for (const [index, part] of left.split(".").map(Number).entries()) {
    const difference = part - (rightParts[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

/**
 * Returns the notes that a frontend build injects. Untagged builds skip the packaging checks, so
 * unreadable notes hide all of them instead of failing the build.
 */
export async function bundleReleaseNotes(root: string): Promise<ReleaseNote[]> {
  try {
    return await readReleaseNotes(root);
  } catch (error) {
    console.warn(`Building without release notes: ${(error as Error).message}`);
    return [];
  }
}

/**
 * Throws unless a versioned package may ship its notes: every file is valid, the version has one, and
 * the directory holds exactly what the checked-out commit holds.
 */
export async function requireReleaseNotes(
  root: string,
  version: string,
): Promise<void> {
  requireNotesOf(await readReleaseNotes(root), version);
  await requireCommittedReleaseNotes(root);
}

/** Rejects notes that differ from HEAD; other local edits stay allowed for patched distributions. */
async function requireCommittedReleaseNotes(root: string): Promise<void> {
  const temporary = await mkdtemp(path.join(tmpdir(), "ora-release-notes-"));
  // Writes nothing to the checkout's Git directory and runs none of its hooks.
  const isolated = [
    ["core.splitIndex", "false"],
    ["core.fsmonitor", "false"],
    ["core.hooksPath", temporary],
  ].flatMap(([key, value]) => ["-c", `${key}=${value}`]);
  /** Runs Git against the packaged checkout, optionally through another index. */
  const git = async (args: string[], index?: string): Promise<string> => {
    const { stdout } = await promisify(execFile)(
      "git",
      ["-C", root, "--no-optional-locks", ...isolated, ...args],
      {
        env:
          index === undefined
            ? process.env
            : { ...process.env, GIT_INDEX_FILE: index },
      },
    );
    return stdout;
  };
  const status = [
    "status",
    "--porcelain",
    "--untracked-files=all",
    "--ignored=matching",
    "--",
    RELEASE_NOTES_DIRECTORY,
  ];
  let changes: string;
  try {
    // A source copy nested in another repository must not be checked against that repository.
    if ((await git(["rev-parse", "--show-prefix"])).trimEnd() !== "") {
      throw new Error("the packaging root is not the Git checkout root");
    }
    // The checkout's own index also reports edits that are only staged.
    changes = await git(status);
    if (changes === "") {
      // A private index that holds only HEAD ignores flags such as assume-unchanged and
      // skip-worktree, which make the checkout's own index hide edits.
      const index = path.join(temporary, "index");
      await git(["read-tree", "HEAD"], index);
      changes = await git(status, index);
    }
  } catch (error) {
    throw new Error(
      `Packaging with --tag requires a Git checkout to verify ${RELEASE_NOTES_DIRECTORY}`,
      { cause: error },
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
  if (changes !== "") {
    throw new Error(
      `${RELEASE_NOTES_DIRECTORY} differs from the checked-out commit:\n${changes.trimEnd()}`,
    );
  }
}
