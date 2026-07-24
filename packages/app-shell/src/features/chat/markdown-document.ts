const WRAPPED_MARKDOWN_PATTERN = /^\s*```(?:markdown|md)\s*\r?\n([\s\S]*?)\r?\n```\s*$/i;

/** Removes a document-level Markdown fence that would otherwise turn the whole response into code. */
export function unwrapMarkdownDocument(content: string): string {
  return content.match(WRAPPED_MARKDOWN_PATTERN)?.[1] ?? content;
}
