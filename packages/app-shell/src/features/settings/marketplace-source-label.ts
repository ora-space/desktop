/**
 * Short, human-readable labels for marketplace sources, derived from their canonical URLs.
 *
 * Registry ADR 20260831 (D6) requires every listing to show where it came from without showing
 * the digest-suffixed namespace. This module is a stopgap for the marketplace cards only: the
 * backend is expected to deliver the label with the contract, comparing against every source it
 * has ever seen, at which point this derivation goes away.
 *
 * The URL is never parsed as `owner/repo` — hosting platforms disagree about path structure
 * (Azure `_git`, GitLab subgroups, Bitbucket `/scm/`), so the label works on plain path segments.
 */

/** Separator shown in place of the segments skipped between the distinguishing one and the name. */
const ELLIPSIS = "…";

/**
 * Splits a source URL into comparable segments: host (with port) first, then each path segment.
 *
 * Case, a trailing slash, and a trailing `.git` do not change which repository a URL names, so
 * they are normalized away before comparison.
 */
function sourceSegments(sourceUrl: string): string[] {
  const trimmed = sourceUrl.trim();
  let segments: string[];
  try {
    const url = new URL(trimmed);
    segments = [url.host, ...url.pathname.split("/")];
  } catch {
    // scp-like Git remotes (`git@host:path`) are not WHATWG URLs; `:` and `/` both separate.
    segments = trimmed.split(/[/:]/);
  }
  segments = segments.map((segment) => segment.toLowerCase()).filter(Boolean);
  const last = segments.length - 1;
  if (last >= 0 && segments[last].endsWith(".git")) {
    segments[last] = segments[last].slice(0, -".git".length);
  }
  return segments.filter(Boolean);
}

/**
 * Labels one source inside a group of sources sharing its last segment: the nearest segment
 * (walking back from the name) that no other member has at the same distance from the end.
 */
function disambiguatedLabel(segments: string[], others: string[][]): string {
  const name = segments[segments.length - 1];
  for (let distance = 1; distance < segments.length; distance += 1) {
    const segment = segments[segments.length - 1 - distance];
    const unique = others.every(
      (other) => other[other.length - 1 - distance] !== segment,
    );
    if (unique) {
      return distance === 1
        ? `${segment}/${name}`
        : `${segment}/${ELLIPSIS}/${name}`;
    }
  }
  // No single segment separates this source from every sibling, so only the whole address does.
  return segments.join("/");
}

/**
 * Derives the display label of every distinct source URL in `sourceUrls`.
 *
 * A label is the repository name (the last path segment). When two sources share that name —
 * typically a fork — each one is prefixed with its first distinguishing segment, so the result
 * depends on which sources are listed together; it is display text, never identity.
 */
export function marketplaceSourceLabels(
  sourceUrls: Iterable<string>,
): Map<string, string> {
  const segmentsByUrl = new Map<string, string[]>();
  for (const url of sourceUrls) {
    if (!segmentsByUrl.has(url)) segmentsByUrl.set(url, sourceSegments(url));
  }

  // Several spellings of one address normalize to the same segments and must not collide with
  // themselves, so each group is keyed by the normalized address.
  const byName = new Map<string, Map<string, string[]>>();
  for (const segments of segmentsByUrl.values()) {
    const name = segments[segments.length - 1] ?? "";
    const group = byName.get(name) ?? new Map<string, string[]>();
    group.set(segments.join("/"), segments);
    byName.set(name, group);
  }

  const labels = new Map<string, string>();
  for (const [url, segments] of segmentsByUrl) {
    const name = segments[segments.length - 1];
    if (name === undefined) {
      labels.set(url, url.trim());
      continue;
    }
    const key = segments.join("/");
    const others = [...(byName.get(name)?.entries() ?? [])]
      .filter(([otherKey]) => otherKey !== key)
      .map(([, other]) => other);
    labels.set(
      url,
      others.length === 0 ? name : disambiguatedLabel(segments, others),
    );
  }
  return labels;
}
