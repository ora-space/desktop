import type { TFunction } from "i18next";
import type { PluginOrigin, SkillSource } from "@ora/contracts";

/**
 * Where a Skill or plugin comes from, as shown to the user.
 *
 * Plugin origins come from the host (`PluginOrigin`), which also computes the marketplace label;
 * the frontend never parses namespaces or URLs (registry ADR 20260831 D6). `user` marks Skills
 * the user created or imported, which belong to no plugin. A local package names its plugin so
 * the tooltip can say which uninstall removes it.
 */
export type OriginDisplay =
  | { kind: "user" }
  | { kind: "local_archive"; pluginName: string }
  | { kind: "official" }
  | { kind: "marketplace"; label: string };

/** Projects a host-computed plugin origin onto its display form. */
export function pluginOriginDisplay(
  origin: PluginOrigin,
  pluginName: string,
): OriginDisplay {
  switch (origin.kind) {
    case "local_archive":
      return { kind: "local_archive", pluginName };
    case "official":
      return { kind: "official" };
    case "marketplace":
      return { kind: "marketplace", label: origin.label };
  }
}

/**
 * Projects a Skill's source onto its display form; a plugin Skill shows its plugin's origin.
 *
 * `pluginName` resolves a plugin id to the name a local package's tooltip shows.
 */
export function skillOriginDisplay(
  source: SkillSource,
  pluginName: (pluginId: string) => string,
): OriginDisplay {
  return source.kind === "local"
    ? { kind: "user" }
    : pluginOriginDisplay(source.origin, pluginName(source.pluginId));
}

/**
 * Chooses the badge text and the one-sentence tooltip for one origin.
 *
 * Rows that keep their own accessible name use the text to describe the origin separately.
 */
export function originCopy(
  origin: OriginDisplay,
  t: TFunction,
): { text: string; description: string } {
  switch (origin.kind) {
    case "user":
      return {
        text: t("plugins.origin.user"),
        description: t("plugins.origin.userDescription"),
      };
    case "local_archive":
      return {
        text: t("plugins.origin.localArchive"),
        description: t("plugins.origin.localArchiveDescription", {
          name: origin.pluginName,
        }),
      };
    case "official":
      return {
        text: t("plugins.origin.official"),
        description: t("plugins.origin.officialDescription"),
      };
    case "marketplace":
      return {
        text: origin.label,
        description: t("plugins.origin.marketplaceDescription"),
      };
  }
}
