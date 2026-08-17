import type { ReactNode } from "react";
import { ChatFileLink } from "./chat-file-link";
import { parseChatHref } from "./parse";

const INLINE_CODE_CLASS =
  "rounded-sm border border-border/70 bg-muted/80 px-1.5 py-[0.15em] font-mono text-[0.85em] text-foreground";

const WEB_LINK_CLASS =
  "font-medium text-primary underline decoration-primary/45 underline-offset-4 transition-colors hover:decoration-primary focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";

/** Reads a single path token from react-markdown inline `code` children. */
function singleTextChild(children: ReactNode): string | null {
  if (typeof children === "string") return children;
  if (typeof children === "number") return String(children);
  if (Array.isArray(children) && children.length === 1) {
    return singleTextChild(children[0]);
  }
  return null;
}

/** Overrides Markdown `code` so path-like inline spans can become chat file links. */
export function ChatMarkdownCode({
  children,
  className,
  ...props
}: {
  children?: ReactNode;
  className?: string;
}) {
  if (className !== undefined) {
    return (
      <code className={className} {...props}>
        {children}
      </code>
    );
  }
  const token = singleTextChild(children);
  if (token === null) {
    return (
      <code className={INLINE_CODE_CLASS} {...props}>
        {children}
      </code>
    );
  }
  return (
    <ChatFileLink source="inline-code" raw={token}>
      {children}
    </ChatFileLink>
  );
}

/** Overrides Markdown `a` so http(s) stay web links and file hrefs use the classifier. */
export function ChatMarkdownAnchor({
  children,
  href,
}: {
  children?: ReactNode;
  href?: string;
}) {
  const parsed = parseChatHref(href ?? "");
  if (parsed.kind === "inert") {
    return (
      <a
        className={WEB_LINK_CLASS}
        href={href}
        onClick={(event) => event.preventDefault()}
      >
        {children}
      </a>
    );
  }
  if (parsed.kind === "web") {
    return (
      <a
        className={WEB_LINK_CLASS}
        href={parsed.href}
        rel="noopener noreferrer"
        target="_blank"
      >
        {children}
      </a>
    );
  }
  return (
    <ChatFileLink source="href" raw={href ?? ""}>
      {children}
    </ChatFileLink>
  );
}
