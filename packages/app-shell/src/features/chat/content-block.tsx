import { useState } from "react";
import {
  IconArrowsMaximize,
  IconDownload,
  IconExternalLink,
  IconFile,
  IconFileText,
  IconPhoto,
  IconVolume,
} from "@tabler/icons-react";
import type { acp } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { ImagePreviewDialog } from "./image-preview-dialog";

interface ContentBlockProps {
  content: Exclude<acp.ContentBlock, { type: "text" }>;
  appearance?: "message" | "tool";
}

/** Renders every structured ACP content block with stable, accessible media controls. */
export function ContentBlock({ content, appearance = "message" }: ContentBlockProps) {
  switch (content.type) {
    case "image":
      return <ImageBlock content={content} appearance={appearance} />;
    case "audio":
      return <AudioBlock content={content} />;
    case "resource_link":
      return <ResourceLinkBlock content={content} />;
    case "resource":
      return <EmbeddedResourceBlock content={content} />;
  }
}

/** Presents image output as inspectable content without allowing it to shift the thread. */
function ImageBlock({
  content,
  appearance,
}: {
  content: acp.ImageContent;
  appearance: "message" | "tool";
}) {
  const { t } = useTranslation();
  const [previewOpen, setPreviewOpen] = useState(false);
  const src = mediaDataUrl(content.mimeType, content.data);
  if (src === null || !PREVIEWABLE_IMAGE_TYPES.has(content.mimeType.toLocaleLowerCase())) {
    return <BinaryDownload uri={content.uri ?? undefined} mimeType={content.mimeType} data={content.data} />;
  }
  const label = content.uri ? resourceName(content.uri) : t("chat.content.generatedImage");
  return (
    <figure className={`overflow-hidden rounded-md border border-border bg-muted/20 ${appearance === "tool" ? "max-w-xl" : "max-w-2xl"}`}>
      <div className="group relative">
        <span className="flex min-h-36 max-h-[32rem] items-center justify-center bg-[var(--code-background)]">
          <img
            src={src}
            alt={label}
            loading="lazy"
            decoding="async"
            className="max-h-[32rem] w-auto max-w-full object-contain"
          />
        </span>
        <button
          type="button"
          data-slot="image-expand-button"
          onClick={() => setPreviewOpen(true)}
          aria-label={t("chat.content.expandImage", { name: label })}
          className="absolute right-2 top-2 flex size-9 cursor-pointer items-center justify-center rounded-md border border-white/15 bg-black/55 text-white/90 opacity-80 shadow-sm outline-none backdrop-blur-sm transition-[opacity,background-color] duration-150 hover:bg-black/70 hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-white"
        >
            <IconArrowsMaximize className="size-4" />
        </button>
      </div>
      <figcaption className="flex min-h-9 items-center gap-2 border-t border-border px-3 text-xs text-muted-foreground">
        <IconPhoto className="size-4 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate" title={label}>{label}</span>
        <span className="shrink-0 font-mono text-[10px]">{content.mimeType}</span>
      </figcaption>
      <ImagePreviewDialog open={previewOpen} src={src} name={label} onOpenChange={setPreviewOpen} />
    </figure>
  );
}

/** Uses the platform audio control so keyboard and assistive-technology behavior stay native. */
function AudioBlock({ content }: { content: acp.AudioContent }) {
  const { t } = useTranslation();
  const src = mediaDataUrl(content.mimeType, content.data);
  if (src === null || !content.mimeType.startsWith("audio/")) {
    return <BinaryDownload mimeType={content.mimeType} data={content.data} />;
  }
  return (
    <section className="max-w-xl rounded-md border border-border bg-muted/20 p-3" aria-label={t("chat.content.audio") }>
      <div className="mb-2 flex items-center gap-2 text-xs font-medium">
        <IconVolume className="size-4 text-sky-600 dark:text-sky-400" aria-hidden="true" />
        <span>{t("chat.content.audio")}</span>
        <span className="ml-auto font-mono text-[10px] font-normal text-muted-foreground">{content.mimeType}</span>
      </div>
      <audio controls preload="metadata" src={src} className="h-10 w-full" />
    </section>
  );
}

/** Shows a linked resource as one descriptive, keyboard-reachable row. */
function ResourceLinkBlock({ content }: { content: acp.ResourceLink }) {
  const title = content.title ?? content.name;
  const href = safeResourceHref(content.uri);
  return (
    <a
      href={href ?? undefined}
      target={href === null ? undefined : "_blank"}
      rel={href === null ? undefined : "noreferrer"}
      className={`flex min-h-11 max-w-2xl items-center gap-3 rounded-md border border-border bg-muted/20 px-3 py-2 outline-none transition-colors duration-200 ${href === null ? "cursor-default" : "hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring"}`}
    >
      <IconFileText className="size-4 shrink-0 text-sky-600 dark:text-sky-400" aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-xs font-medium" title={title}>{title}</span>
        {content.description && <span className="mt-0.5 block line-clamp-2 text-[11px] leading-4 text-muted-foreground">{content.description}</span>}
        <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground" title={content.uri}>{content.uri}</span>
      </span>
      {content.size != null && <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">{formatBytes(content.size)}</span>}
      <IconExternalLink className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
    </a>
  );
}

/** Keeps embedded source readable while progressively disclosing potentially long payloads. */
function EmbeddedResourceBlock({ content }: { content: acp.EmbeddedResource }) {
  const resource = content.resource;
  if ("text" in resource) {
    const name = resourceName(resource.uri);
    return (
      <details className="group max-w-2xl overflow-hidden rounded-md border border-border bg-muted/20" open>
        <summary className="flex min-h-11 cursor-pointer list-none items-center gap-2 px-3 text-xs outline-none transition-colors duration-200 hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
          <IconFileText className="size-4 shrink-0 text-sky-600 dark:text-sky-400" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate font-medium" title={resource.uri}>{name}</span>
          {resource.mimeType && <span className="shrink-0 font-mono text-[10px] text-muted-foreground">{resource.mimeType}</span>}
        </summary>
        <pre data-selectable className="max-h-80 overflow-auto border-t border-border bg-[var(--code-background)] px-3 py-2.5 text-[11px] leading-5 whitespace-pre-wrap">{resource.text}</pre>
      </details>
    );
  }
  if (resource.mimeType && PREVIEWABLE_IMAGE_TYPES.has(resource.mimeType.toLocaleLowerCase())) {
    return <ImageBlock content={{ data: resource.blob, mimeType: resource.mimeType, uri: resource.uri }} appearance="message" />;
  }
  if (resource.mimeType?.startsWith("audio/")) {
    return <AudioBlock content={{ data: resource.blob, mimeType: resource.mimeType }} />;
  }
  return <BinaryDownload uri={resource.uri} mimeType={resource.mimeType ?? undefined} data={resource.blob} />;
}

/** Provides a safe fallback for binary resources the browser cannot preview inline. */
function BinaryDownload({ uri, mimeType, data }: { uri?: string; mimeType?: string; data: string }) {
  const { t } = useTranslation();
  const src = mediaDataUrl(mimeType ?? "application/octet-stream", data);
  const name = uri ? resourceName(uri) : t("chat.content.binaryResource");
  return (
    <a
      href={src ?? undefined}
      download={name}
      className="flex min-h-11 max-w-2xl items-center gap-3 rounded-md border border-border bg-muted/20 px-3 py-2 outline-none transition-colors duration-200 hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring"
    >
      <IconFile className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate text-xs font-medium" title={name}>{name}</span>
      {mimeType && <span className="shrink-0 font-mono text-[10px] text-muted-foreground">{mimeType}</span>}
      <IconDownload className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="sr-only">{t("chat.content.download", { name })}</span>
    </a>
  );
}

/** Constructs a media URL only for syntactically valid base64 payloads and MIME types. */
function mediaDataUrl(mimeType: string, data: string): string | null {
  if (!/^[\w.+-]+\/[\w.+-]+$/.test(mimeType) || !/^[A-Za-z0-9+/]*={0,2}$/.test(data)) return null;
  return `data:${mimeType};base64,${data}`;
}

const PREVIEWABLE_IMAGE_TYPES = new Set([
  "image/avif",
  "image/bmp",
  "image/gif",
  "image/jpeg",
  "image/png",
  "image/webp",
]);

/** Rejects executable URI schemes while preserving ACP-specific resource schemes. */
function safeResourceHref(uri: string): string | null {
  const trimmed = uri.trim();
  return /^(?:javascript|data|vbscript):/i.test(trimmed) ? null : trimmed;
}

/** Derives a compact display name from URL-like and filesystem-like resource identifiers. */
function resourceName(uri: string): string {
  const withoutQuery = uri.split(/[?#]/, 1)[0] ?? uri;
  const tail = withoutQuery.split(/[\\/]/).filter(Boolean).at(-1);
  if (tail === undefined) return uri;
  try {
    return decodeURIComponent(tail);
  } catch {
    return tail;
  }
}

/** Formats protocol byte counts without narrowing potentially large bigint values. */
function formatBytes(bytes: bigint): string {
  const units = ["B", "KB", "MB", "GB", "TB"] as const;
  let value = bytes;
  let unit = 0;
  while (value >= 1024n && unit < units.length - 1) {
    value /= 1024n;
    unit += 1;
  }
  return `${value.toLocaleString()} ${units[unit]}`;
}
