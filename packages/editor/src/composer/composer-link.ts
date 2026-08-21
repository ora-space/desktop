import Link from "@tiptap/extension-link";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { sanitizeUrl } from "../utils";

/**
 * Schemes the host browser command will open. Keep in sync with Desktop
 * `open_external::is_browser_url` so pointerdown does not swallow a press for a
 * URL the Rust side will reject.
 */
export function isComposerOpenableUrl(href: string): boolean {
  if (href.length === 0 || /\s/.test(href)) {
    return false;
  }
  const lower = href.toLowerCase();
  return (
    lower.startsWith("https://") ||
    lower.startsWith("http://") ||
    lower.startsWith("mailto:")
  );
}

/**
 * Schemes that must never become a link mark or agent payload, even before
 * `sanitizeUrl` (which needs a window base URL). Kept window-free so the
 * Markdown parser can reuse this in Node tests.
 */
export function isDangerousComposerHref(href: string): boolean {
  return /^(?:javascript|data|vbscript|file):/i.test(href.trim());
}

/**
 * Returns a browser-openable href, or null when the URL is javascript/data/etc.
 * or a scheme Desktop will not launch.
 */
export function safeComposerHref(href: string): string | null {
  const safe = sanitizeUrl(href, window.location.href);
  if (safe === "#" || !isComposerOpenableUrl(safe)) {
    return null;
  }
  return safe;
}

/**
 * Resolves a left-button press on a composer `<a href>` to a safe external URL.
 * `root` keeps nested/editor-chrome anchors from being treated as prompt links.
 */
export function resolveComposerLinkHref(
  event: { button?: number; target: EventTarget | null },
  root?: ParentNode | null,
): string | null {
  if (event.button !== undefined && event.button !== 0) {
    return null;
  }
  const target = event.target;
  if (!(target instanceof Element)) {
    return null;
  }
  const link = target.closest("a[href]");
  if (
    !(link instanceof HTMLAnchorElement) ||
    (root !== undefined && root !== null && !root.contains(link)) ||
    link.getAttribute("href") === "#"
  ) {
    return null;
  }
  return safeComposerHref(link.href);
}

/**
 * Opens http(s) links from an editable prompt. Tiptap's built-in handler uses
 * `window.open` on `click` (mouseup); pointerdown starts navigation immediately
 * so contenteditable caret placement does not swallow the first press.
 */
function openComposerHref(href: string): void {
  window.open(href, "_blank", "noopener,noreferrer");
}

/**
 * Dashboard-style underlined links. Click opens a new tab; `inclusive` is off
 * so typing after the link is body text.
 */
export const ComposerLink = Link.extend({
  inclusive() {
    return false;
  },

  addProseMirrorPlugins() {
    return [
      ...(this.parent?.() ?? []),
      new Plugin({
        key: new PluginKey("composerLinkOpen"),
        props: {
          handleDOMEvents: {
            pointerdown: (view, event) => {
              const href = resolveComposerLinkHref(event, view.dom);
              if (href === null) {
                return false;
              }
              event.preventDefault();
              openComposerHref(href);
              return true;
            },
          },
        },
      }),
    ];
  },
}).configure({
  // ComposerLink opens on pointerdown; Tiptap's click opener would double-fire
  // and wait until mouseup, which is what made the first press feel dead.
  openOnClick: false,
  autolink: true,
  linkOnPaste: true,
  markdownLinks: true,
  // `defaultProtocol` / `markdownLinks` need @tiptap/extension-link >= 3.26,
  // which is why that package's specifier is higher than the other @tiptap/*.
  defaultProtocol: "https",
  HTMLAttributes: {
    target: "_blank",
    rel: "noopener noreferrer nofollow",
    class: "composer-link",
  },
});
