/** Converts ripgrep's one-based UTF-8 byte column into a JavaScript UTF-16 string index. */
export function utf8ByteColumnToStringIndex(value: string, column: number): number {
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
