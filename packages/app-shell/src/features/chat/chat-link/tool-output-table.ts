export interface PowerShellTableLayout {
  modeStart: number;
  nameStart: number;
}

export interface PowerShellModeEntry {
  kind: "file" | "directory";
  path: string;
}

/** Column order of a `Select-Object Name, PSIsContainer` listing. */
export interface PowerShellContainerLayout {
  nameFirst: boolean;
}

/**
 * True for a header underline such as `----`. PowerShell prints one under every
 * column, and a mode column of a plain file is dashes too, so the name side is
 * what separates a real row from the rule under the header.
 */
function isTableRule(value: string): boolean {
  return /^-+$/.test(value);
}

/** Removes terminal styling without changing visible table column widths. */
export function stripAnsi(value: string): string {
  const visible: string[] = [];
  for (let index = 0; index < value.length; index++) {
    if (value.charCodeAt(index) !== 0x1b || value[index + 1] !== "[") {
      visible.push(value[index]!);
      continue;
    }
    index += 2;
    while (index < value.length) {
      const code = value.charCodeAt(index);
      if (code >= 0x40 && code <= 0x7e) break;
      index++;
    }
  }
  return visible.join("");
}

/** Parses PowerShell `Mode Name` or `Name Mode` rows without accepting table headers. */
export function powerShellModeEntry(
  rawLine: string,
  layout?: PowerShellTableLayout | null,
): PowerShellModeEntry | null {
  if (layout !== null && layout !== undefined) {
    const mode = rawLine
      .slice(layout.modeStart)
      .match(/^(d[-a-z]{4,}|-[-a-z]{4,})/i)?.[1];
    const path =
      layout.nameStart > layout.modeStart
        ? rawLine.slice(layout.nameStart).trim()
        : rawLine.slice(layout.nameStart, layout.modeStart).trim();
    if (mode !== undefined && path !== "" && !isTableRule(path)) {
      return {
        kind: mode.toLowerCase().startsWith("d") ? "directory" : "file",
        path,
      };
    }
  }
  const trimmed = rawLine.trim();
  const modeFirst = trimmed.match(/^(d[-a-z]{4,}|-[-a-z]{4,})\s+(.+?)\s*$/i);
  if (modeFirst !== null && !isTableRule(modeFirst[2]!)) {
    return {
      kind: modeFirst[1]!.toLowerCase().startsWith("d") ? "directory" : "file",
      path: modeFirst[2]!,
    };
  }
  const nameFirst = trimmed.match(/^(.+?)\s+(d[-a-z]{4,}|-[-a-z]{4,})\s*$/i);
  if (nameFirst === null || isTableRule(nameFirst[1]!)) return null;
  return {
    kind: nameFirst[2]!.toLowerCase().startsWith("d") ? "directory" : "file",
    path: nameFirst[1]!,
  };
}

/**
 * Locates a `Name` / `PSIsContainer` header. `Get-ChildItem | Select-Object
 * Name, PSIsContainer` is a common way to list a directory, and its True/False
 * column is the same explicit kind evidence a `Mode` column carries.
 */
export function powerShellContainerLayout(
  lines: readonly string[],
): PowerShellContainerLayout | null {
  for (const line of lines) {
    const nameStart = line.search(/\bName\b/i);
    const containerStart = line.search(/\bPSIsContainer\b/i);
    if (nameStart === -1 || containerStart === -1) continue;
    return { nameFirst: nameStart < containerStart };
  }
  return null;
}

/**
 * Parses one `Name` / `PSIsContainer` row. The header gate matters: a bare
 * `something True` line in arbitrary output is not a directory listing.
 * Boolean columns are right-aligned, so the row is matched by its trailing
 * token rather than by header column offsets, which a long name overflows.
 */
export function powerShellContainerEntry(
  rawLine: string,
  layout?: PowerShellContainerLayout | null,
): PowerShellModeEntry | null {
  if (layout === null || layout === undefined) return null;
  const trimmed = rawLine.trim();
  const match = layout.nameFirst
    ? trimmed.match(/^(.*\S)\s+(True|False)$/i)
    : trimmed.match(/^(True|False)\s+(.*\S)$/i);
  if (match === null) return null;
  const path = (layout.nameFirst ? match[1] : match[2])!;
  const container = (layout.nameFirst ? match[2] : match[1])!;
  if (path === "" || isTableRule(path)) return null;
  return {
    kind: container.toLowerCase() === "true" ? "directory" : "file",
    path,
  };
}

/** Locates Mode and Name columns in aligned PowerShell table output. */
export function powerShellTableLayout(
  lines: readonly string[],
): PowerShellTableLayout | null {
  for (const line of lines) {
    const modeStart = line.search(/\bMode\b/i);
    const nameStart = line.search(/\bName\b/i);
    if (modeStart !== -1 && nameStart !== -1) return { modeStart, nameStart };
  }
  return null;
}

/**
 * Builds the typed-row reader for one output block. A block carries at most one
 * table shape, so the layouts are resolved once and reused for every line.
 */
export function powerShellEntryReader(
  lines: readonly string[],
): (line: string) => PowerShellModeEntry | null {
  const tableLayout = powerShellTableLayout(lines);
  const containerLayout = powerShellContainerLayout(lines);
  return (line) =>
    powerShellModeEntry(line, tableLayout) ??
    powerShellContainerEntry(line, containerLayout);
}
