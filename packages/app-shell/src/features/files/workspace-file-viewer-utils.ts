/** Browser default tab advance inside `<pre>` (CSS `tab-size: 8`). */
const TAB_COLUMNS = 8;

/**
 * Estimates one line's rendered width in viewer columns: tabs advance to the
 * next tab stop and full-width scripts occupy roughly two cells. The viewer
 * sizes the scrollable area from the widest line so the horizontal scrollbar
 * stays stable while only a window of lines is mounted; wide scripts are
 * over-counted on purpose since a clipped line is worse than a little extra
 * scroll room.
 */
export function lineDisplayColumns(line: string): number {
  let columns = 0;
  for (let index = 0; index < line.length; index += 1) {
    const code = line.charCodeAt(index);
    if (code === 0x09) {
      columns += TAB_COLUMNS - (columns % TAB_COLUMNS);
      continue;
    }
    columns += isWideCodePoint(code) ? 2 : 1;
  }
  return columns;
}

/** Whether one UTF-16 code unit renders as a wide cell in a monospace run. */
function isWideCodePoint(code: number): boolean {
  return (
    (code >= 0x1100 && code <= 0x115f) ||
    (code >= 0x2e80 && code <= 0xa4cf) ||
    (code >= 0xac00 && code <= 0xd7a3) ||
    (code >= 0xf900 && code <= 0xfaff) ||
    (code >= 0xff01 && code <= 0xff60) ||
    (code >= 0xffe0 && code <= 0xffe6)
  );
}

/** Converts ripgrep's one-based UTF-8 byte column into a JavaScript UTF-16 string index. */
export function utf8ByteColumnToStringIndex(
  value: string,
  column: number,
): number {
  const targetOffset = Math.max(0, column - 1);
  let byteOffset = 0;
  let stringIndex = 0;
  for (const character of value) {
    const width = utf8Width(character.codePointAt(0) ?? 0);
    if (byteOffset + width > targetOffset) break;
    byteOffset += width;
    stringIndex += character.length;
  }
  return stringIndex;
}

/** Returns the encoded width of one Unicode scalar value in UTF-8. */
function utf8Width(codePoint: number): number {
  if (codePoint <= 0x7f) return 1;
  if (codePoint <= 0x7ff) return 2;
  if (codePoint <= 0xffff) return 3;
  return 4;
}
