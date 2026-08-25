import type { ComposerFileAttrs } from "./composer-file";

/** `start:end:path` info string of a file-preview citation fence. */
const CITATION_INFO = /^(\d+):(\d+):(.+)$/;
/** First line of the mini patch a diff-gutter quote expands to. */
const DIFF_FILE_HEADER = /^diff --git a\/(.+) b\/(.+)$/;
/** Hunk header note; the side is absent when one chip spans add and delete. */
const DIFF_HUNK_HEADER =
  /^@@ -\d+,\d+ \+\d+,\d+ @@ quoted from git diff(?: \((old|new) side\))?, lines (\d+)-(\d+)$/;

/**
 * Reads a fenced quote payload back into the chip attrs it was expanded from.
 *
 * Sending a prompt flattens chips through `composerFilePlainText`, so surfaces
 * that only hold the sent text — chat history — need the inverse to show the
 * same chip the composer did. Returns null for every other fence so ordinary
 * code blocks keep rendering as code.
 *
 * @param info Fence info string (language plus meta, joined by a space).
 * @param body Fence contents without the surrounding markers.
 */
export function parseComposerFileQuote(
  info: string,
  body: string,
): ComposerFileAttrs | null {
  const citation = CITATION_INFO.exec(info);
  if (citation !== null) {
    const [, startLine, endLine, path] = citation;
    if (
      startLine === undefined ||
      endLine === undefined ||
      path === undefined
    ) {
      return null;
    }
    return {
      path,
      startLine: Number(startLine),
      endLine: Number(endLine),
      snippet: body,
      kind: "file",
      // Explicit so both branches return the same shape as
      // `composerFileAttrsFromUnknown`, which chip surfaces compare against.
      origin: undefined,
      diffSide: undefined,
    };
  }
  return info === "diff" ? parseDiffQuote(body) : null;
}

/**
 * The patch is only a quote when all four header lines are the ones
 * `diffQuotePlainText` writes; a hand-pasted diff stays a code block.
 */
function parseDiffQuote(body: string): ComposerFileAttrs | null {
  const lines = body.split("\n");
  const header = DIFF_FILE_HEADER.exec(lines[0] ?? "");
  if (header === null) return null;
  const [, oldPath, newPath] = header;
  if (oldPath === undefined || newPath === undefined || oldPath !== newPath) {
    return null;
  }
  if (lines[1] !== `--- a/${oldPath}` || lines[2] !== `+++ b/${newPath}`) {
    return null;
  }
  const hunk = DIFF_HUNK_HEADER.exec(lines[3] ?? "");
  if (hunk === null) return null;
  const [, diffSide, startLine, endLine] = hunk;
  if (startLine === undefined || endLine === undefined) return null;
  return {
    path: newPath,
    startLine: Number(startLine),
    endLine: Number(endLine),
    snippet: lines.slice(4).join("\n"),
    kind: "file",
    origin: "diff",
    diffSide: diffSide === "old" || diffSide === "new" ? diffSide : undefined,
  };
}
