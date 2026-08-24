import type { AnchorHTMLAttributes, ReactNode } from "react";
import { useOptionalPlatform } from "../../platform";

const HTTP_HREF = /^https?:/i;

export interface ChatExternalLinkProps extends Omit<
  AnchorHTMLAttributes<HTMLAnchorElement>,
  "target" | "rel"
> {
  href: string;
  children?: ReactNode;
}

/**
 * Renders a chat `http(s)` link that always leaves the webview.
 *
 * Desktop's main window registers no `on_new_window` hook (only plugin surface
 * webviews do, see `surface/hooks.rs`), so a plain `<a target="_blank">` click
 * is silently dropped there even though it opens fine in a real browser tab
 * and in every component test run under jsdom. Route the click through the
 * same `openExternalUrl` command the prompt box already uses instead of
 * relying on native new-window handling.
 */
export function ChatExternalLink({
  href,
  children,
  ...props
}: ChatExternalLinkProps) {
  const platform = useOptionalPlatform();
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      {...props}
      onClick={(event) => {
        props.onClick?.(event);
        if (platform === null || !HTTP_HREF.test(href)) {
          return;
        }
        event.preventDefault();
        void platform.openExternalUrl(href);
      }}
    >
      {children}
    </a>
  );
}
