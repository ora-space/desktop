import type { InstalledPlugin, Skill } from "@ora/contracts";

/** The runtime keeps unavailable entries as candidates, so they can still cause ambiguity. */
export type WorkflowSkillResolution =
  | { status: "missing" }
  | { status: "ambiguous"; candidates: readonly Skill[] }
  | { status: "installed" | "unavailable"; skill: Skill };

/** Rust's eq_ignore_ascii_case intentionally does not fold non-ASCII characters. */
function asciiLower(value: string): string {
  return value.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
}

/** Mirrors resolve_graph_ref_skill_entry; canonical identities never fall back to names. */
export function resolveWorkflowSkill(
  reference: string,
  skills: readonly Skill[],
  retiredIds: readonly string[] = [],
): WorkflowSkillResolution {
  if (retiredIds.includes(reference)) return { status: "missing" };
  const exact = skills.find((skill) => skill.id === reference);
  if (exact) {
    return {
      status: exact.availability === "available" ? "installed" : "unavailable",
      skill: exact,
    };
  }
  if (reference.startsWith("plugin:")) return { status: "missing" };
  const pluginReference =
    !reference.includes(":") && reference.split("/").length === 2;
  const name = asciiLower(reference.split(":").at(-1)!);
  const candidates = skills.filter((skill) =>
    pluginReference
      ? skill.namespace === reference
      : asciiLower(skill.name) === name,
  );
  if (candidates.length === 0) return { status: "missing" };
  if (candidates.length > 1) return { status: "ambiguous", candidates };
  const skill = candidates[0]!;
  return {
    status: skill.availability === "available" ? "installed" : "unavailable",
    skill,
  };
}

/** Names the source even when the installed plugin record itself is unavailable. */
export function workflowSkillLabel(
  skill: Skill,
  plugins: readonly InstalledPlugin[],
  localLabel: string,
): string {
  const source = skill.source;
  if (source.kind === "local") return `${skill.name} · ${localLabel}`;
  const plugin = plugins.find((candidate) => candidate.id === source.pluginId);
  const label = plugin?.displayName;
  const origin =
    label && label !== source.pluginId
      ? `${label} (${source.pluginId})`
      : source.pluginId;
  return `${skill.name} · ${origin}`;
}

/** Extracts the install identity instead of searching the market for a catalog ID. */
export function workflowSkillInstallQuery(reference: string): string {
  if (!reference.startsWith("plugin:")) return reference;
  const suffix = reference.slice("plugin:".length);
  const separator = suffix.lastIndexOf(":");
  return separator < 0 ? suffix : suffix.slice(0, separator);
}
