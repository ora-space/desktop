export interface PowerShellTableLayout {
  modeStart: number;
  nameStart: number;
}

export interface PowerShellModeEntry {
  kind: "file" | "directory";
  path: string;
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
    if (mode !== undefined && path !== "") {
      return {
        kind: mode.toLowerCase().startsWith("d") ? "directory" : "file",
        path,
      };
    }
  }
  const trimmed = rawLine.trim();
  const modeFirst = trimmed.match(/^(d[-a-z]{4,}|-[-a-z]{4,})\s+(.+?)\s*$/i);
  if (modeFirst !== null) {
    return {
      kind: modeFirst[1]!.toLowerCase().startsWith("d") ? "directory" : "file",
      path: modeFirst[2]!,
    };
  }
  const nameFirst = trimmed.match(/^(.+?)\s+(d[-a-z]{4,}|-[-a-z]{4,})\s*$/i);
  if (nameFirst === null) return null;
  return {
    kind: nameFirst[2]!.toLowerCase().startsWith("d") ? "directory" : "file",
    path: nameFirst[1]!,
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
