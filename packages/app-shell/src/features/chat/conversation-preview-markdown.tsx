import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { unwrapMarkdownDocument } from "./markdown-document";

interface ConversationPreviewMarkdownProps {
  content: string;
}

const previewMarkdownComponents: Components = {
  a: ({ children }) => <span className="underline decoration-foreground/30 underline-offset-2">{children}</span>,
  blockquote: ({ children }) => <div className="my-1 border-l border-foreground/20 pl-2 text-foreground/80">{children}</div>,
  code: ({ children, className }) => className === undefined
    ? <code className="rounded-sm bg-muted px-1 py-0.5 font-mono text-[0.9em] text-foreground">{children}</code>
    : <code className="whitespace-pre-wrap break-words font-mono text-foreground/85">{children}</code>,
  h1: ({ children }) => <p className="font-semibold">{children}</p>,
  h2: ({ children }) => <p className="font-semibold">{children}</p>,
  h3: ({ children }) => <p className="font-semibold">{children}</p>,
  h4: ({ children }) => <p className="font-semibold">{children}</p>,
  h5: ({ children }) => <p className="font-semibold">{children}</p>,
  h6: ({ children }) => <p className="font-semibold">{children}</p>,
  hr: () => <span className="my-1 block h-px bg-foreground/15" />,
  img: ({ alt }) => <span>{alt}</span>,
  li: ({ children }) => <li className="pl-0.5">{children}</li>,
  ol: ({ children }) => <ol className="my-0.5 list-decimal pl-4">{children}</ol>,
  p: ({ children }) => <p className="my-0.5 first:mt-0 last:mb-0">{children}</p>,
  pre: ({ children }) => (
    <div
      data-preview-code-block
      className="my-0.5 overflow-hidden border-l-2 border-foreground/15 pl-2 font-mono text-[11px] leading-[18px] text-foreground/85"
    >
      {children}
    </div>
  ),
  table: ({ children }) => <table className="my-1 w-full table-fixed text-[11px]">{children}</table>,
  td: ({ children }) => <td className="truncate border-t border-foreground/10 pr-1">{children}</td>,
  th: ({ children }) => <th className="truncate pr-1 text-left font-medium">{children}</th>,
  ul: ({ children }) => <ul className="my-0.5 list-disc pl-4">{children}</ul>,
};

/** Renders a bounded Markdown subset suited to the navigator's non-interactive preview. */
export function ConversationPreviewMarkdown({ content }: ConversationPreviewMarkdownProps) {
  const markdown = unwrapMarkdownDocument(content);
  return (
    <div className="max-h-15 overflow-hidden text-xs leading-5 break-words [overflow-wrap:anywhere]">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={previewMarkdownComponents}>{markdown}</ReactMarkdown>
    </div>
  );
}
