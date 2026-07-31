import remend, {
  isWithinCodeBlock,
  isWithinLinkOrImageUrl,
  isWithinMathBlock,
  type RemendHandler,
} from "remend";

const AMBIGUOUS_INLINE_DELIMITER_PATTERN = /(?:\*+|_+|~+|`+)$/;
const AMBIGUOUS_BLOCK_MARKER_PATTERN = /^[ \t]{0,3}(?:#{1,6}|>|[-+*]|\d{1,9}[.)])[ \t]*$/;

const pendingBoundaryHandler: RemendHandler = {
  name: "pending-markdown-boundary",
  priority: -1,
  handle: withholdPendingBoundary,
};

/**
 * Repairs an incomplete Markdown snapshot while withholding only its still-ambiguous final boundary.
 *
 * The source snapshot remains untouched outside this derived render path. Once another character
 * resolves the syntax—or the caller stops using this streaming path—the boundary is rendered
 * normally, so literal marker characters can never be lost.
 */
export function prepareStreamingMarkdown(markdown: string) {
  return remend(markdown, { handlers: [pendingBoundaryHandler] });
}

/** Removes a syntactically undecidable tail before Remend completes the remaining Markdown. */
function withholdPendingBoundary(markdown: string) {
  const lineStart = markdown.lastIndexOf("\n") + 1;
  const lastLine = markdown.slice(lineStart);
  if (
    AMBIGUOUS_BLOCK_MARKER_PATTERN.test(lastLine)
    && !isWithinCodeBlock(markdown, lineStart)
    && !isWithinMathBlock(markdown, lineStart)
  ) {
    return markdown.slice(0, lineStart);
  }

  const delimiterMatch = markdown.match(AMBIGUOUS_INLINE_DELIMITER_PATTERN);
  if (!delimiterMatch) return markdown;
  const delimiterStart = markdown.length - delimiterMatch[0].length;
  let escapeCount = 0;
  for (let index = delimiterStart - 1; index >= 0 && markdown[index] === "\\"; index -= 1) {
    escapeCount += 1;
  }
  if (
    escapeCount % 2 === 1
    || isWithinCodeBlock(markdown, delimiterStart)
    || isWithinMathBlock(markdown, delimiterStart)
    || isWithinLinkOrImageUrl(markdown, delimiterStart)
  ) {
    return markdown;
  }
  return markdown.slice(0, delimiterStart);
}
