import { isValidElement, useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { IconCheck, IconChevronsDown, IconChevronsUp, IconCopy } from "@tabler/icons-react";
import { Button } from "@ora/ui";
import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import { useTranslation } from "react-i18next";
import remarkGfm from "remark-gfm";
import type { BundledLanguage, ThemedTokenWithVariants } from "shiki";

interface MarkdownMessageProps {
  content: string;
}

const WRAPPED_MARKDOWN_PATTERN = /^\s*```(?:markdown|md)\s*\r?\n([\s\S]*?)\r?\n```\s*$/i;
const LANGUAGE_CLASS_PATTERN = /(?:^|\s)language-([^\s]+)/;
const highlightedCodeCache = new Map<string, Promise<ThemedTokenWithVariants[][] | null>>();

interface ShikiTokenStyle extends CSSProperties {
  "--shiki-dark"?: string;
}

const markdownComponents: Components = {
  a: ({ children, ...props }) => (
    <a
      className="font-medium text-primary underline decoration-primary/45 underline-offset-4 transition-colors hover:decoration-primary focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      rel="noopener noreferrer"
      target="_blank"
      {...props}
    >
      {children}
    </a>
  ),
  blockquote: ({ children, ...props }) => (
    <blockquote className="my-3 border-l-2 border-border pl-3 text-muted-foreground" {...props}>
      {children}
    </blockquote>
  ),
  code: ({ children, className, ...props }) => {
    const inlineClassName = className === undefined
      ? "rounded-sm bg-muted px-1.5 py-0.5 font-mono text-[0.85em]"
      : className;
    return <code className={inlineClassName} {...props}>{children}</code>;
  },
  h1: ({ children, ...props }) => <h1 className="mb-3 mt-6 text-2xl font-semibold leading-8 first:mt-0" {...props}>{children}</h1>,
  h2: ({ children, ...props }) => <h2 className="mb-2 mt-5 text-xl font-semibold leading-8 first:mt-0" {...props}>{children}</h2>,
  h3: ({ children, ...props }) => <h3 className="mb-2 mt-4 text-lg font-semibold leading-7 first:mt-0" {...props}>{children}</h3>,
  hr: (props) => <hr className="my-4 border-border" {...props} />,
  li: ({ children, ...props }) => <li className="my-1 pl-1" {...props}>{children}</li>,
  ol: ({ children, ...props }) => <ol className="my-3 list-decimal space-y-1 pl-6 marker:text-muted-foreground" {...props}>{children}</ol>,
  p: ({ children, ...props }) => <p className="my-3 first:mt-0 last:mb-0" {...props}>{children}</p>,
  pre: ({ children }) => {
    if (isValidElement<{ children?: ReactNode; className?: string }>(children)) {
      const language = children.props.className?.match(LANGUAGE_CLASS_PATTERN)?.[1] ?? "text";
      return <CodeBlock code={String(children.props.children).replace(/\n$/, "")} language={language} />;
    }
    return <pre>{children}</pre>;
  },
  table: ({ children, ...props }) => (
    <div className="my-3 max-w-full overflow-x-auto rounded-md border border-border/70">
      <table className="w-max min-w-full border-collapse text-left text-[13px] leading-5" {...props}>{children}</table>
    </div>
  ),
  td: ({ children, ...props }) => <td className="border-t border-border/70 px-3 py-2 align-top" {...props}>{children}</td>,
  th: ({ children, ...props }) => <th className="bg-muted/55 px-3 py-2 font-medium" {...props}>{children}</th>,
  ul: ({ children, ...props }) => <ul className="my-3 list-disc space-y-1 pl-6 marker:text-muted-foreground" {...props}>{children}</ul>,
};

/** Renders untrusted assistant Markdown without enabling raw HTML execution. */
export function MarkdownMessage({ content }: MarkdownMessageProps) {
  const markdown = unwrapMarkdownDocument(content);
  return (
    <div className="min-w-0 break-words text-[15px] leading-[26px] text-foreground">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{markdown}</ReactMarkdown>
    </div>
  );
}

/** Removes a document-level Markdown fence that would otherwise turn the whole response into code. */
function unwrapMarkdownDocument(content: string): string {
  return content.match(WRAPPED_MARKDOWN_PATTERN)?.[1] ?? content;
}

/** Wraps fenced code with persistent copy and disclosure controls. */
function CodeBlock({ code, language }: { code: string; language: string }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(true);
  const [copied, setCopied] = useState(false);
  const lineCount = code === "" ? 0 : code.split(/\r?\n/).length;

  const copyCode = () => {
    navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div
      data-expanded={expanded}
      className={`markdown-code-block my-3 max-w-full overflow-hidden rounded-r-md border-l-2 ${
        expanded
          ? "border-foreground/45 bg-white dark:border-border dark:bg-[var(--code-background)]"
          : "border-border bg-[var(--code-background)]"
      }`}
    >
      <div className="flex min-h-9 items-center px-3">
        <span className="font-mono text-[11px] font-medium text-muted-foreground">{language}</span>
        <span className="mx-2 h-3 w-px bg-border" aria-hidden="true" />
        <span className="text-[11px] text-muted-foreground" aria-live="polite">
          {expanded
            ? t("chat.codeLineCount", { count: lineCount })
            : t("chat.codeLinesCollapsed", { count: lineCount })}
        </span>
        <div className="ml-auto flex items-center gap-0.5">
          <Button variant="ghost" size="icon-xs" onClick={copyCode} aria-label={copied ? t("chat.codeCopied") : t("chat.copyCode")}>
            {copied ? <IconCheck className="size-3.5 text-emerald-600" /> : <IconCopy className="size-3.5" />}
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            onClick={() => setExpanded((current) => !current)}
            aria-expanded={expanded}
            aria-label={expanded ? t("chat.collapseCode") : t("chat.expandCode")}
          >
            {expanded ? <IconChevronsUp className="size-3.5" /> : <IconChevronsDown className="size-3.5" />}
          </Button>
        </div>
      </div>
      {expanded && (
        <pre className="max-w-full overflow-x-auto px-4 pb-3 pt-2 font-mono text-[13px] leading-6">
          <HighlightedCode code={code} language={language} />
        </pre>
      )}
    </div>
  );
}

/** Highlights fenced code with VS Code's TextMate grammars and paired default themes. */
function HighlightedCode({ code, language }: { code: string; language: string }) {
  const [tokens, setTokens] = useState<ThemedTokenWithVariants[][] | null>(null);

  useEffect(() => {
    let active = true;
    const cacheKey = `${language}\u0000${code}`;
    let pending = highlightedCodeCache.get(cacheKey);
    if (pending === undefined) {
      pending = import("shiki")
        .then(({ codeToTokensWithThemes }) => codeToTokensWithThemes(code, {
          lang: language as BundledLanguage,
          themes: { light: "light-plus", dark: "dark-plus" },
        }))
        .catch(() => null);
      highlightedCodeCache.set(cacheKey, pending);
    }
    pending.then((nextTokens) => {
      if (active) setTokens(nextTokens);
    });
    return () => {
      active = false;
    };
  }, [code, language]);

  if (tokens === null) return <code>{code}</code>;
  return (
    <code>
      {tokens.map((line, lineIndex) => (
        <span key={lineIndex} className="block min-h-6">
          {line.map((token, tokenIndex) => {
            const light = token.variants.light;
            const dark = token.variants.dark;
            const style: ShikiTokenStyle = { color: light?.color, "--shiki-dark": dark?.color };
            return <span key={`${tokenIndex}-${token.offset}`} className="shiki-token" style={style}>{token.content}</span>;
          })}
        </span>
      ))}
    </code>
  );
}
