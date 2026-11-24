// Release notes are handwritten per stable version in release-notes/<version>.json. This module owns
// their format.
import { lstat, readFile, readdir } from "node:fs/promises";
import path from "node:path";

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
// Bidirectional embeddings, overrides, and isolates can make text read differently from its content.
const forbiddenTextPattern = /[\p{Cc}\u202A-\u202E\u2066-\u2069]/u;

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
            `${label}.${locale} contains a control or bidirectional formatting character`,
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
    notes.push(
      parseReleaseNote(
        file.name,
        await readFile(path.join(directory, file.name), "utf8"),
      ),
    );
  }
  return notes;
}
