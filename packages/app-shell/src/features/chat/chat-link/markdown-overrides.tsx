import {
  Children,
  cloneElement,
  isValidElement,
  type ComponentPropsWithoutRef,
  type ReactNode,
} from "react";
import { ChatExternalLink } from "../chat-external-link";
import { ChatFileLink } from "./chat-file-link";
import { isPathLikeToken, parseChatHref } from "./parse";
import { useChatLinkContext } from "./context";
import type { SessionArtifactIndex } from "./artifact-index";
import {
  isPlainPathList,
  pathTokenFromOutputLine,
  stripListMarker,
} from "./tool-output-paths";

const INLINE_CODE_CLASS =
  "rounded-sm border border-border/70 bg-muted/80 px-1.5 py-[0.15em] font-mono text-[0.85em] text-foreground";

const WEB_LINK_CLASS =
  "font-medium text-primary underline decoration-primary/45 underline-offset-4 transition-colors hover:decoration-primary focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";

const artifactBasenameCache = new WeakMap<SessionArtifactIndex, Set<string>>();

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
  if (token === null || token.includes("\n")) {
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
    return <>{children}</>;
  }
  if (parsed.kind === "web") {
    return (
      <ChatExternalLink className={WEB_LINK_CLASS} href={parsed.href}>
        {children}
      </ChatExternalLink>
    );
  }
  return (
    <ChatFileLink source="href" raw={href ?? ""}>
      {children}
    </ChatFileLink>
  );
}

/** Overrides Markdown `p` so list/prose path tokens that hit the index become links. */
export function ChatMarkdownParagraph({
  children,
  ...props
}: ComponentPropsWithoutRef<"p">) {
  const artifactIndex = useChatLinkContext()?.index;
  return (
    <p className="my-3 first:mt-0 last:mb-0" {...props}>
      {linkifyChildren(children, artifactIndex)}
    </p>
  );
}

/** Tight lists put the path text on the `li` itself instead of a nested `p`. */
export function ChatMarkdownListItem({
  children,
  ...props
}: ComponentPropsWithoutRef<"li">) {
  const artifactIndex = useChatLinkContext()?.index;
  return (
    <li className="my-1 pl-1" {...props}>
      {linkifyChildren(children, artifactIndex)}
    </li>
  );
}

/** File tables should be as clickable as bullet lists of the same paths. */
export function ChatMarkdownTableCell({
  children,
  ...props
}: ComponentPropsWithoutRef<"td">) {
  const artifactIndex = useChatLinkContext()?.index;
  return (
    <td className="border-t border-border/70 px-3 py-2 align-top" {...props}>
      {linkifyChildren(children, artifactIndex)}
    </td>
  );
}

/** Shared line renderer so path fences and tool dumps cannot drift apart. */
function PathLinkLines({
  text,
  lineClassName,
}: {
  text: string;
  lineClassName: string;
}) {
  const artifactIndex = useChatLinkContext()?.index;
  return (
    <>
      {text.split(/\r?\n/).map((line, index) => {
        const stripped = stripListMarker(line);
        const token = pathTokenFromOutputLine(line) ?? stripped;
        const wholeLineLink =
          token !== "" &&
          shouldAttemptArtifactLink(token, artifactIndex) &&
          (token !== stripped || !/\s/.test(stripped));
        return (
          <div key={index} className={lineClassName}>
            {wholeLineLink ? (
              <ChatFileLink source="inline-code" raw={token} unmatched="text">
                {line}
              </ChatFileLink>
            ) : (
              linkifyText(line, artifactIndex) || "\u00a0"
            )}
          </div>
        );
      })}
    </>
  );
}

/** Renders a fenced path list as chat file links instead of highlighted source. */
export function ChatPathListBlock({ code }: { code: string }) {
  return (
    <div
      data-testid="chat-path-list"
      className="my-3 max-w-full overflow-x-auto rounded-r-md border-l-2 border-border bg-[var(--code-background)] px-4 py-3 font-mono text-[13px] leading-6"
    >
      <PathLinkLines
        text={code}
        lineClassName="min-h-[1.5em] whitespace-pre-wrap"
      />
    </div>
  );
}

/** Makes glob/search dump lines clickable without restyling non-path output. */
export function ChatToolOutputText({ text }: { text: string }) {
  return (
    <div
      data-selectable
      data-testid="chat-tool-path-output"
      className="max-h-72 overflow-auto rounded-r-sm border-l-2 border-border bg-[var(--code-background)] px-3 py-2.5 font-mono text-[11px] leading-5"
    >
      <PathLinkLines
        text={text}
        lineClassName="min-h-[1.25em] whitespace-pre-wrap"
      />
    </div>
  );
}

/** Chooses a clickable path list for plaintext fences, otherwise the host code block. */
export function ChatMarkdownPre({
  children,
  renderCodeBlock,
}: ComponentPropsWithoutRef<"pre"> & {
  renderCodeBlock: (code: string, language: string) => ReactNode;
}) {
  const artifactIndex = useChatLinkContext()?.index;
  const fenced = fencedCodeFromPreChild(children);
  if (fenced !== null) {
    if (
      isPlainPathList(fenced.code, fenced.language) ||
      isIndexedArtifactList(fenced.code, fenced.language, artifactIndex)
    ) {
      return <ChatPathListBlock code={fenced.code} />;
    }
    return renderCodeBlock(fenced.code, fenced.language);
  }
  return <pre>{children}</pre>;
}

/** Recognizes bare fenced lists only when every entry is known to this conversation. */
function isIndexedArtifactList(
  code: string,
  language: string,
  artifactIndex: SessionArtifactIndex | undefined,
): boolean {
  if (
    artifactIndex === undefined ||
    (language !== "text" && language !== "plaintext")
  ) {
    return false;
  }
  const lines = code
    .split(/\r?\n/)
    .map(stripListMarker)
    .filter((line) => line !== "");
  const tokens = lines.flatMap(artifactTextTokens);
  return (
    tokens.length > 1 &&
    tokens.filter((token) => shouldAttemptArtifactLink(token, artifactIndex))
      .length >= 2
  );
}

/** Reads the fenced code string and language from a Markdown `pre > code` child. */
function fencedCodeFromPreChild(children: ReactNode): {
  code: string;
  language: string;
} | null {
  const child = Array.isArray(children)
    ? children.find(isValidElement)
    : children;
  if (!isValidElement<{ children?: ReactNode; className?: string }>(child)) {
    return null;
  }
  const language =
    child.props.className?.match(/(?:^|\s)language-([^\s]+)/)?.[1] ?? "text";
  return {
    code: fencedChildText(child.props.children).replace(/\n$/, ""),
    language,
  };
}

/** Flattens react-markdown's fenced `code` children into one source string. */
function fencedChildText(children: ReactNode): string {
  if (typeof children === "string" || typeof children === "number") {
    return String(children);
  }
  if (Array.isArray(children)) return children.map(fencedChildText).join("");
  if (isValidElement<{ children?: ReactNode }>(children)) {
    return fencedChildText(children.props.children);
  }
  return "";
}

/** Walks markdown children and wraps path-like text that hits the session index. */
function linkifyChildren(
  children: ReactNode,
  artifactIndex: SessionArtifactIndex | undefined,
): ReactNode {
  return Children.map(children, (child) => {
    if (typeof child === "string" || typeof child === "number") {
      return linkifyText(String(child), artifactIndex);
    }
    if (!isValidElement<{ children?: ReactNode }>(child)) return child;
    if (shouldSkipLinkify(child.type)) return child;
    return cloneElement(
      child,
      undefined,
      linkifyChildren(child.props.children, artifactIndex),
    );
  });
}

/** Skips nodes that already own linking so overrides are not wrapped again. */
function shouldSkipLinkify(type: unknown): boolean {
  return (
    type === "code" ||
    type === "a" ||
    type === "pre" ||
    type === "p" ||
    type === ChatFileLink ||
    type === ChatMarkdownCode ||
    type === ChatMarkdownAnchor ||
    type === ChatMarkdownParagraph ||
    type === ChatMarkdownListItem ||
    type === ChatMarkdownTableCell ||
    type === ChatMarkdownPre
  );
}

/** Scans prose tokens while keeping a natural-language line suffix attached. */
function linkifyText(
  text: string,
  artifactIndex: SessionArtifactIndex | undefined,
): ReactNode {
  const stripped = stripListMarker(text);
  if (stripped !== "" && isPathLikeToken(stripped) && !/\s/.test(stripped)) {
    return (
      <ChatFileLink source="inline-code" raw={stripped} unmatched="text">
        {text}
      </ChatFileLink>
    );
  }
  const mentions = artifactTextMatches(text);
  if (mentions.length === 0) return text;

  const rendered: ReactNode[] = [];
  let offset = 0;
  for (const [mentionIndex, mention] of mentions.entries()) {
    const start = mention.index ?? offset;
    const token = mention[0];
    if (start > offset) rendered.push(text.slice(offset, start));
    rendered.push(
      shouldAttemptArtifactLink(token, artifactIndex) ? (
        <ChatFileLink
          key={mentionIndex}
          source="inline-code"
          raw={token}
          unmatched="text"
        >
          {token}
        </ChatFileLink>
      ) : (
        token
      ),
    );
    offset = start + token.length;
  }
  if (offset < text.length) rendered.push(text.slice(offset));
  return rendered;
}

/** Splits prose/list columns on whitespace and common list punctuation. */
function artifactTextMatches(text: string): RegExpMatchArray[] {
  return [
    ...text.matchAll(
      /[^\s,，、;；]+(?:\s+\(line\s+[1-9]\d*(?:\s*,\s*col(?:umn)?\s+[1-9]\d*)?\))?/gi,
    ),
  ];
}

/** Returns candidate token text without discarding source offsets used by rendering. */
function artifactTextTokens(text: string): string[] {
  return artifactTextMatches(text).map((match) => match[0]);
}

/** Uses path syntax or an O(1) known basename hit before mounting a link component. */
function shouldAttemptArtifactLink(
  raw: string,
  artifactIndex: SessionArtifactIndex | undefined,
): boolean {
  if (isPathLikeToken(raw)) return true;
  if (artifactIndex === undefined) return false;
  let basenames = artifactBasenameCache.get(artifactIndex);
  if (basenames === undefined) {
    basenames = new Set(
      [
        ...artifactIndex.edited,
        ...artifactIndex.referenced,
        ...(artifactIndex.directories ?? []),
        ...(artifactIndex.unknown ?? []),
      ].map((path) =>
        path
          .replaceAll("\\", "/")
          .replace(/\/+$/, "")
          .split("/")
          .at(-1)!
          .toLowerCase(),
      ),
    );
    artifactBasenameCache.set(artifactIndex, basenames);
  }
  const token = raw
    .replace(/[,;.，；。！？、：]+$/, "")
    .replace(/[\\/]+$/, "")
    .toLowerCase();
  return basenames.has(token);
}
