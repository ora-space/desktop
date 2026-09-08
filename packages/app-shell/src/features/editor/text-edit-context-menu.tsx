import {
  cloneElement,
  useRef,
  useState,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  cn,
} from "@ora/ui";

interface TextEditContextMenuProps {
  children: ReactNode;
  /** Host element merged onto the trigger so WebKit/Tauri still deliver `contextmenu`. */
  trigger: ReactElement<{
    className?: string;
    onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
  }>;
  /** When false, Cut and Paste stay visible but disabled (read-only transcript). */
  editable: boolean;
  hasSelection: boolean;
  onOpenChange?: (open: boolean) => void;
  onCut: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onSelectAll: () => void;
}

/**
 * Writes plain text for Copy and Cut. Callers serialize their own selection
 * because a composer chip and a Markdown transcript are not the same payload.
 */
export async function writeClipboardText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // WebView may deny clipboard write; Copy/Cut already closed the menu.
  }
}

/**
 * Reads clipboard text for Paste. A denied or empty clipboard is an empty
 * string so the editor does not insert a failed-read diagnostic.
 */
export async function readClipboardText(): Promise<string> {
  try {
    return await navigator.clipboard.readText();
  } catch {
    return "";
  }
}

/**
 * Reads image files from the clipboard. ClipboardItem has `getType`, not the
 * DataTransfer `getAsFiles` helper — that mismatch is why menu Paste saw no
 * images while Ctrl+V (which reads `clipboardData.files`) still worked.
 */
export async function readClipboardFiles(): Promise<File[]> {
  if (typeof navigator.clipboard.read !== "function") {
    return [];
  }
  try {
    const items = await navigator.clipboard.read();
    const files: File[] = [];
    for (const item of items) {
      for (const type of item.types) {
        if (!type.startsWith("image/")) {
          continue;
        }
        const blob = await item.getType(type);
        const subtype = type.slice("image/".length).split(";")[0] ?? "png";
        files.push(new File([blob], `clipboard.${subtype}`, { type }));
      }
    }
    return files;
  } catch {
    return [];
  }
}

/** Selects every node inside `element` so a later Copy uses the visible transcript. */
export function selectElementContents(element: HTMLElement): void {
  const selection = window.getSelection();
  if (selection === null) {
    return;
  }
  const range = document.createRange();
  range.selectNodeContents(element);
  selection.removeAllRanges();
  selection.addRange(range);
}

/**
 * Resolves a right-clicked image, including overlay buttons and preview canvases
 * whose `<img>` has `pointer-events-none`.
 */
function copyableImageFromEvent(
  event: MouseEvent<HTMLElement>,
): HTMLImageElement | null {
  const node = event.target;
  if (!(node instanceof Element)) {
    return null;
  }
  if (node instanceof HTMLImageElement && node.src !== "") {
    return node;
  }
  const host = node.closest("[data-copyable-image]");
  if (host === null) {
    return null;
  }
  if (host instanceof HTMLImageElement && host.src !== "") {
    return host;
  }
  const nested = host.querySelector("img");
  return nested instanceof HTMLImageElement && nested.src !== ""
    ? nested
    : null;
}

/** Decodes a data URL so ACP/composer images can be written without a network fetch. */
function blobFromDataUrl(src: string): Blob | null {
  const match = /^data:([^;,]+);base64,(.+)$/i.exec(src);
  if (match === null) {
    return null;
  }
  let binary: string;
  try {
    binary = atob(match[2] ?? "");
  } catch {
    return null;
  }
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return new Blob([bytes], { type: match[1] ?? "image/png" });
}

/** Rasterizes `img` to PNG. WebView2 rejects ClipboardItem JPEG payloads. */
function blobFromCanvas(img: HTMLImageElement): Promise<Blob | null> {
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const context = canvas.getContext("2d");
  if (context === null || canvas.width === 0 || canvas.height === 0) {
    return Promise.resolve(null);
  }
  context.drawImage(img, 0, 0);
  return new Promise((resolve) => {
    canvas.toBlob((blob) => resolve(blob), "image/png");
  });
}

/**
 * Copies the displayed bitmap. Prefer selecting the `<img>` and using the
 * same native copy path as Ctrl+C; ClipboardItem write is a fallback and must
 * be PNG or WebView2 silently drops it.
 */
export async function copyImageElement(img: HTMLImageElement): Promise<void> {
  if (copyImageViaNativeSelection(img)) {
    return;
  }
  if (img.decode !== undefined) {
    try {
      await img.decode();
    } catch {
      // Decode failure still allows a canvas attempt from the current frame.
    }
  }
  const fromDataUrl = blobFromDataUrl(img.src);
  const png =
    fromDataUrl?.type === "image/png" ? fromDataUrl : await blobFromCanvas(img);
  if (png === null || typeof ClipboardItem === "undefined") {
    return;
  }
  try {
    await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
  } catch {
    try {
      await navigator.clipboard.write([
        new ClipboardItem({ "image/png": Promise.resolve(png) }),
      ]);
    } catch {
      // Neither ClipboardItem construction path is available in this WebView.
    }
  }
}

/**
 * User-gesture copy of an already-rendered image. Chromium still honors this
 * when `clipboard.write` of a ClipboardItem is denied to the WebView.
 */
function copyImageViaNativeSelection(img: HTMLImageElement): boolean {
  const selection = window.getSelection();
  if (selection === null) {
    return false;
  }
  const previous: Range[] = [];
  for (let index = 0; index < selection.rangeCount; index += 1) {
    previous.push(selection.getRangeAt(index));
  }
  const range = document.createRange();
  range.selectNode(img);
  selection.removeAllRanges();
  selection.addRange(range);
  let copied = false;
  try {
    copied = document.execCommand("copy");
  } finally {
    selection.removeAllRanges();
    for (const restored of previous) {
      selection.addRange(restored);
    }
  }
  return copied;
}

/**
 * Cursor-style Cut / Copy / Paste / Select All. The trigger is a `div` via
 * `render` so the default button trigger cannot swallow `contextmenu` in the
 * desktop WebView; `select-text` overrides the shared trigger's `select-none`.
 * Right-clicking a `[data-copyable-image]` host copies the bitmap instead of text.
 */
export function TextEditContextMenu({
  children,
  trigger,
  editable,
  hasSelection,
  onOpenChange,
  onCut,
  onCopy,
  onPaste,
  onSelectAll,
}: TextEditContextMenuProps) {
  const { t } = useTranslation();
  const parkedImageRef = useRef<HTMLImageElement | null>(null);
  const [hasImage, setHasImage] = useState(false);
  const cutDisabled = !editable || !hasSelection;
  const copyDisabled = !hasSelection && !hasImage;
  const pasteDisabled = !editable;

  return (
    <ContextMenu onOpenChange={onOpenChange}>
      <ContextMenuTrigger
        className="select-text"
        render={cloneElement(trigger, {
          className: cn(trigger.props.className, "select-text"),
          onContextMenu: (event) => {
            event.preventDefault();
            const image = copyableImageFromEvent(event);
            parkedImageRef.current = image;
            setHasImage(image !== null);
            trigger.props.onContextMenu?.(event);
          },
        })}
      >
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-36">
        <ContextMenuItem disabled={cutDisabled} onClick={onCut}>
          {t("chat.cut")}
        </ContextMenuItem>
        <ContextMenuItem
          disabled={copyDisabled}
          onClick={() => {
            const image = parkedImageRef.current;
            if (image !== null) {
              void copyImageElement(image);
              return;
            }
            onCopy();
          }}
        >
          {t("chat.copy")}
        </ContextMenuItem>
        <ContextMenuItem disabled={pasteDisabled} onClick={onPaste}>
          {t("chat.paste")}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onClick={onSelectAll}>
          {t("chat.selectAll")}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
