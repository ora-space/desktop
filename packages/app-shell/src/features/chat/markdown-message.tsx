import {
  isValidElement,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ComponentPropsWithoutRef,
  type ReactNode,
  type RefObject,
} from "react";
import {
  IconCheck,
  IconChevronsDown,
  IconChevronsUp,
  IconCopy,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import { useTranslation } from "react-i18next";
import remarkGfm from "remark-gfm";
import type { BundledLanguage, ThemedTokenWithVariants } from "shiki";
import { unwrapMarkdownDocument } from "./markdown-document";
import { prepareStreamingMarkdown } from "./streaming-markdown";
import {
  ChatMarkdownAnchor,
  ChatMarkdownCode,
  ChatMarkdownListItem,
  ChatMarkdownParagraph,
  ChatMarkdownPre,
  ChatMarkdownTableCell,
} from "./chat-link/markdown-overrides";
import { useChatLinkContext } from "./chat-link/context";

interface MarkdownMessageProps {
  content: string;
  streaming?: boolean;
}

interface MarkdownDocumentProps {
  content: string;
  components?: Components;
}

const LANGUAGE_CLASS_PATTERN = /(?:^|\s)language-([^\s]+)/;
const markdownRemarkPlugins = [remarkGfm];
const highlightedCodeCache = new Map<
  string,
  Promise<ThemedTokenWithVariants[][] | null>
>();

interface ShikiTokenStyle extends CSSProperties {
  "--shiki-dark"?: string;
}

interface MarkdownAstPosition {
  start: { offset?: number };
  end: { offset?: number };
}

interface MarkdownAstNode {
  type: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: MarkdownAstNode[];
  position?: MarkdownAstPosition;
}

interface StreamingRevealState {
  renderedLength: number;
  revealStart: number;
  streaming: boolean;
}

interface StreamingRevealSpanProps extends ComponentPropsWithoutRef<"span"> {
  node?: MarkdownAstNode;
  registryRef: RefObject<Set<HTMLElement>>;
  "data-stream-text-reveal"?: boolean;
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
    <blockquote
      className="my-3 border-l-2 border-border pl-3 text-muted-foreground"
      {...props}
    >
      {children}
    </blockquote>
  ),
  code: ({ children, className, ...props }) => {
    const inlineClassName =
      className === undefined
        ? "rounded-sm border border-border/70 bg-muted/80 px-1.5 py-[0.15em] font-mono text-[0.85em] text-foreground"
        : className;
    return (
      <code className={inlineClassName} {...props}>
        {children}
      </code>
    );
  },
  h1: ({ children, ...props }) => (
    <h1
      className="mb-3 mt-6 text-2xl font-semibold leading-8 first:mt-0"
      {...props}
    >
      {children}
    </h1>
  ),
  h2: ({ children, ...props }) => (
    <h2
      className="mb-2 mt-5 text-xl font-semibold leading-8 first:mt-0"
      {...props}
    >
      {children}
    </h2>
  ),
  h3: ({ children, ...props }) => (
    <h3
      className="mb-2 mt-4 text-lg font-semibold leading-7 first:mt-0"
      {...props}
    >
      {children}
    </h3>
  ),
  hr: (props) => <hr className="my-4 border-border" {...props} />,
  li: ({ children, ...props }) => (
    <li className="my-1 pl-1" {...props}>
      {children}
    </li>
  ),
  ol: ({ children, ...props }) => (
    <ol
      className="my-3 list-decimal space-y-1 pl-6 marker:text-muted-foreground"
      {...props}
    >
      {children}
    </ol>
  ),
  p: ({ children, ...props }) => (
    <p className="my-3 first:mt-0 last:mb-0" {...props}>
      {children}
    </p>
  ),
  pre: ({ children }) => {
    if (
      isValidElement<{ children?: ReactNode; className?: string }>(children)
    ) {
      const language =
        children.props.className?.match(LANGUAGE_CLASS_PATTERN)?.[1] ?? "text";
      return (
        <CodeBlock
          code={String(children.props.children).replace(/\n$/, "")}
          language={language}
        />
      );
    }
    return <pre>{children}</pre>;
  },
  table: ({ children, ...props }) => (
    <div className="my-3 max-w-full overflow-x-auto rounded-md border border-border/70">
      <table
        className="w-max min-w-full border-collapse text-left text-[13px] leading-5"
        {...props}
      >
        {children}
      </table>
    </div>
  ),
  td: ({ children, ...props }) => (
    <td className="border-t border-border/70 px-3 py-2 align-top" {...props}>
      {children}
    </td>
  ),
  th: ({ children, ...props }) => (
    <th className="bg-muted/55 px-3 py-2 font-medium" {...props}>
      {children}
    </th>
  ),
  ul: ({ children, ...props }) => (
    <ul
      className="my-3 list-disc space-y-1 pl-6 marker:text-muted-foreground"
      {...props}
    >
      {children}
    </ul>
  ),
};

/** Stable `pre` override so index updates do not remount CodeBlock state. */
function ChatMarkdownPreOverride({
  children,
}: ComponentPropsWithoutRef<"pre">) {
  return (
    <ChatMarkdownPre renderCodeBlock={renderChatMarkdownCodeBlock}>
      {children}
    </ChatMarkdownPre>
  );
}

/** Module-level callback so ChatMarkdownPreOverride keeps a stable component type. */
function renderChatMarkdownCodeBlock(code: string, language: string) {
  return <CodeBlock code={code} language={language} />;
}

/** Renders raw, non-streaming Markdown on the same safe GFM and visual foundation as chat. */
export function MarkdownDocument({
  content,
  components,
}: MarkdownDocumentProps) {
  const mergedComponents = useMemo(
    () =>
      components === undefined
        ? markdownComponents
        : { ...markdownComponents, ...components },
    [components],
  );
  return (
    <div
      data-selectable
      className="min-w-0 break-words text-[15px] leading-[26px] text-foreground"
    >
      <ReactMarkdown
        remarkPlugins={markdownRemarkPlugins}
        components={mergedComponents}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}

/** Renders untrusted assistant Markdown without enabling raw HTML execution. */
export function MarkdownMessage({
  content,
  streaming = false,
}: MarkdownMessageProps) {
  const markdown = unwrapMarkdownDocument(content);
  const chatLink = useChatLinkContext();
  const markdownWithLinks = useMemo(
    (): Components =>
      chatLink === null
        ? markdownComponents
        : {
            ...markdownComponents,
            a: ChatMarkdownAnchor,
            code: ChatMarkdownCode,
            p: ChatMarkdownParagraph,
            li: ChatMarkdownListItem,
            td: ChatMarkdownTableCell,
            pre: ChatMarkdownPreOverride,
          },
    [chatLink],
  );
  const renderedMarkdown = useFrameBatchedMarkdown(markdown, streaming);
  const parseableMarkdown = useMemo(
    () =>
      streaming ? prepareStreamingMarkdown(renderedMarkdown) : renderedMarkdown,
    [renderedMarkdown, streaming],
  );
  const [storedRevealState, setStoredRevealState] =
    useState<StreamingRevealState>(() => ({
      renderedLength: renderedMarkdown.length,
      revealStart: streaming ? 0 : renderedMarkdown.length,
      streaming,
    }));
  let revealState = storedRevealState;
  if (
    storedRevealState.renderedLength !== renderedMarkdown.length ||
    storedRevealState.streaming !== streaming
  ) {
    // Chat content is append-only while streaming. Length is therefore enough
    // to retain the prior boundary without rescanning an ever-growing string.
    revealState = {
      renderedLength: renderedMarkdown.length,
      revealStart:
        streaming && renderedMarkdown.length >= storedRevealState.renderedLength
          ? storedRevealState.renderedLength
          : renderedMarkdown.length,
      streaming,
    };
    setStoredRevealState(revealState);
  }
  const revealPlugin = useMemo(
    () => createStreamingRevealPlugin(revealState.revealStart),
    [revealState.revealStart],
  );
  const revealNodesRef = useRef(new Set<HTMLElement>());
  const streamingMarkdownComponents = useMemo<Components>(
    () => ({
      ...markdownWithLinks,
      span: (props) => (
        <StreamingRevealSpan {...props} registryRef={revealNodesRef} />
      ),
    }),
    [markdownWithLinks],
  );
  const rehypePlugins = useMemo(
    () => (streaming ? [revealPlugin] : []),
    [revealPlugin, streaming],
  );
  const markdownBody = useMemo(
    () => (
      <ReactMarkdown
        remarkPlugins={markdownRemarkPlugins}
        rehypePlugins={rehypePlugins}
        components={streaming ? streamingMarkdownComponents : markdownWithLinks}
      >
        {parseableMarkdown}
      </ReactMarkdown>
    ),
    [
      parseableMarkdown,
      rehypePlugins,
      streaming,
      streamingMarkdownComponents,
      markdownWithLinks,
    ],
  );

  useLayoutEffect(() => {
    if (streaming) animateStreamingReveal(revealNodesRef.current);
  }, [renderedMarkdown, streaming]);

  return (
    <div
      data-selectable
      className="min-w-0 break-words text-[15px] leading-[26px] text-foreground"
    >
      {markdownBody}
    </div>
  );
}

/** Coalesces stream chunks per frame while guaranteeing progress during continuous output. */
function useFrameBatchedMarkdown(markdown: string, streaming: boolean) {
  const [renderedMarkdown, setRenderedMarkdown] = useState(markdown);
  const latestMarkdownRef = useRef(markdown);
  const committedMarkdownRef = useRef(markdown);
  const frameRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    latestMarkdownRef.current = markdown;
    if (committedMarkdownRef.current === markdown || frameRef.current !== null)
      return;

    const scheduleFrame =
      window.requestAnimationFrame ??
      ((callback: FrameRequestCallback) =>
        window.setTimeout(() => callback(performance.now()), 16));
    frameRef.current = scheduleFrame(() => {
      frameRef.current = null;
      const latestMarkdown = latestMarkdownRef.current;
      committedMarkdownRef.current = latestMarkdown;
      setRenderedMarkdown(latestMarkdown);
    });
  }, [markdown]);

  useEffect(
    () => () => {
      if (frameRef.current === null) return;
      if (window.cancelAnimationFrame)
        window.cancelAnimationFrame(frameRef.current);
      else window.clearTimeout(frameRef.current);
    },
    [],
  );

  return streaming ? renderedMarkdown : markdown;
}

/** Creates one Markdown transform that marks only newly streamed prose for reveal animation. */
function createStreamingRevealPlugin(revealStart: number) {
  return () => (tree: MarkdownAstNode) => {
    wrapStreamingText(tree, revealStart);
  };
}

/** Wraps text after the previous stream boundary while leaving code structures untouched. */
function wrapStreamingText(node: MarkdownAstNode, revealStart: number) {
  if (
    node.tagName === "code" ||
    node.tagName === "pre" ||
    node.children === undefined
  )
    return;
  for (let index = node.children.length - 1; index >= 0; index -= 1) {
    const child = node.children[index]!;
    const childEndOffset = child.position?.end.offset;
    // Source-ordered HAST lets us stop as soon as we reach stable content,
    // keeping reveal work proportional to the newest streamed suffix.
    if (childEndOffset !== undefined && childEndOffset <= revealStart) break;
    if (child.type !== "text" || child.value === undefined) {
      wrapStreamingText(child, revealStart);
      continue;
    }

    const startOffset = child.position?.start.offset;
    if (startOffset === undefined || childEndOffset === undefined) continue;

    const sourceLength = childEndOffset - startOffset;
    // Entities and escapes can make source offsets diverge from visible text.
    // Revealing the whole node is preferable to hiding freshly streamed text
    // behind an unsafe proportional offset.
    const splitIndex =
      revealStart <= startOffset || sourceLength !== child.value.length
        ? 0
        : Math.min(child.value.length, revealStart - startOffset);
    const stableText = child.value.slice(0, splitIndex);
    const revealedText = child.value.slice(splitIndex);
    if (revealedText === "") continue;
    const revealNode: MarkdownAstNode = {
      type: "element",
      tagName: "span",
      properties: { dataStreamTextReveal: true },
      children: [{ ...child, value: revealedText }],
    };
    node.children.splice(
      index,
      1,
      ...(stableText === ""
        ? [revealNode]
        : [{ ...child, value: stableText }, revealNode]),
    );
  }
}

/** Animates the latest prose batch and releases compositor resources when it settles. */
function animateStreamingReveal(nodes: ReadonlySet<HTMLElement>) {
  if (typeof HTMLElement.prototype.animate !== "function") return;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  nodes.forEach((node) => {
    node.getAnimations().forEach((animation) => animation.cancel());
    const animation = node.animate([{ opacity: 0.2 }, { opacity: 1 }], {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
    });
    animation.addEventListener("finish", () => animation.cancel(), {
      once: true,
    });
  });
}

/** Registers one animated Markdown span without rescanning the complete message DOM. */
function StreamingRevealSpan({
  node,
  registryRef,
  "data-stream-text-reveal": reveal,
  ...props
}: StreamingRevealSpanProps) {
  const spanRef = useRef<HTMLSpanElement>(null);
  const shouldReveal =
    reveal === true || node?.properties?.dataStreamTextReveal === true;

  useLayoutEffect(() => {
    const span = spanRef.current;
    if (!shouldReveal || span === null) return;
    const registry = registryRef.current;
    registry.add(span);
    return () => {
      registry.delete(span);
    };
  }, [registryRef, shouldReveal]);

  return (
    <span
      ref={spanRef}
      data-stream-text-reveal={shouldReveal || undefined}
      {...props}
    />
  );
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
      <div data-selection-control className="flex min-h-9 items-center px-3">
        <span className="font-mono text-[11px] font-medium text-muted-foreground">
          {language}
        </span>
        <span className="mx-2 h-3 w-px bg-border" aria-hidden="true" />
        <span className="text-[11px] text-muted-foreground" aria-live="polite">
          {expanded
            ? t("chat.codeLineCount", { count: lineCount })
            : t("chat.codeLinesCollapsed", { count: lineCount })}
        </span>
        <div className="ml-auto flex items-center gap-0.5">
          <Button
            variant="ghost"
            size="icon-xs"
            onClick={copyCode}
            aria-label={copied ? t("chat.codeCopied") : t("chat.copyCode")}
          >
            {copied ? (
              <IconCheck className="size-3.5 text-emerald-600" />
            ) : (
              <IconCopy className="size-3.5" />
            )}
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            onClick={() => setExpanded((current) => !current)}
            aria-expanded={expanded}
            aria-label={
              expanded ? t("chat.collapseCode") : t("chat.expandCode")
            }
          >
            {expanded ? (
              <IconChevronsUp className="size-3.5" />
            ) : (
              <IconChevronsDown className="size-3.5" />
            )}
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
function HighlightedCode({
  code,
  language,
}: {
  code: string;
  language: string;
}) {
  const [tokens, setTokens] = useState<ThemedTokenWithVariants[][] | null>(
    null,
  );

  useEffect(() => {
    let active = true;
    const cacheKey = `${language}\u0000${code}`;
    let pending = highlightedCodeCache.get(cacheKey);
    if (pending === undefined) {
      pending = import("shiki")
        .then(({ codeToTokensWithThemes }) =>
          codeToTokensWithThemes(code, {
            lang: language as BundledLanguage,
            themes: { light: "light-plus", dark: "dark-plus" },
          }),
        )
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
            const style: ShikiTokenStyle = {
              color: light?.color,
              "--shiki-dark": dark?.color,
            };
            return (
              <span
                key={`${tokenIndex}-${token.offset}`}
                className="shiki-token"
                style={style}
              >
                {token.content}
              </span>
            );
          })}
        </span>
      ))}
    </code>
  );
}
