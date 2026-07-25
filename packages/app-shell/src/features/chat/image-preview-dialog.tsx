import { useCallback, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { IconMinus, IconPlus, IconX } from "@tabler/icons-react";
import {
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@ora/ui";
import { useTranslation } from "react-i18next";

interface ImagePreviewDialogProps {
  open: boolean;
  src: string;
  name: string;
  onOpenChange: (open: boolean) => void;
}

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 4;
const ZOOM_STEP = 0.1;

interface DragOrigin {
  pointerId: number;
  clientX: number;
  clientY: number;
  panX: number;
  panY: number;
}

interface PanOffset {
  x: number;
  y: number;
}

/** Displays an image in a keyboard-accessible lightbox with bounded wheel zoom. */
export function ImagePreviewDialog({ open, src, name, onOpenChange }: ImagePreviewDialogProps) {
  const { t } = useTranslation();
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<PanOffset>({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const canvasRef = useRef<HTMLDivElement>(null);
  const removeWheelListenerRef = useRef<(() => void) | null>(null);
  const dragOriginRef = useRef<DragOrigin | null>(null);

  /** Applies a fixed zoom step so buttons, keys, and wheel input share the same limits. */
  const changeZoom = (direction: -1 | 1) => {
    setZoom((current) => clampZoom(current + direction * ZOOM_STEP));
  };

  /** Binds wheel zoom when the portaled canvas actually mounts and removes it on unmount. */
  const bindCanvas = useCallback((canvas: HTMLDivElement | null) => {
    removeWheelListenerRef.current?.();
    removeWheelListenerRef.current = null;
    canvasRef.current = canvas;
    if (canvas === null) return;
    // A non-passive listener is required here because some WebViews otherwise
    // apply native scrolling after React handles the same wheel gesture.
    const zoomWithoutScrolling = (event: WheelEvent) => {
      event.preventDefault();
      event.stopPropagation();
      setZoom((current) => clampZoom(current + (event.deltaY < 0 ? ZOOM_STEP : -ZOOM_STEP)));
    };
    canvas.addEventListener("wheel", zoomWithoutScrolling, { passive: false });
    removeWheelListenerRef.current = () => canvas.removeEventListener("wheel", zoomWithoutScrolling);
  }, []);

  /** Offers keyboard equivalents for the visible zoom controls. */
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "+" || event.key === "=") {
      event.preventDefault();
      changeZoom(1);
    } else if (event.key === "-") {
      event.preventDefault();
      changeZoom(-1);
    } else if (event.key === "0") {
      event.preventDefault();
      setZoom(1);
      setPan({ x: 0, y: 0 });
    }
  };

  /** Starts direct manipulation with the primary button and captures movement outside the canvas. */
  const handlePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    dragOriginRef.current = {
      pointerId: event.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
      panX: pan.x,
      panY: pan.y,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDragging(true);
    event.preventDefault();
  };

  /** Pans the enlarged image in lockstep with the captured pointer. */
  const handlePointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const origin = dragOriginRef.current;
    if (origin === null || origin.pointerId !== event.pointerId) return;
    setPan({
      x: origin.panX + event.clientX - origin.clientX,
      y: origin.panY + event.clientY - origin.clientY,
    });
  };

  /** Ends panning without letting a released pointer leave the canvas stuck in a pressed state. */
  const stopDragging = (event: PointerEvent<HTMLDivElement>) => {
    if (dragOriginRef.current?.pointerId !== event.pointerId) return;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    dragOriginRef.current = null;
    setDragging(false);
  };

  const zoomPercent = Math.round(zoom * 100);
  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          setZoom(1);
          setPan({ x: 0, y: 0 });
        }
        onOpenChange(nextOpen);
      }}
    >
      <DialogContent
        showCloseButton={false}
        className="grid-rows-[3.25rem_1fr] gap-0 overflow-hidden rounded-xl border border-border/40 bg-background/60 p-0 text-foreground shadow-2xl ring-1 ring-foreground/5 backdrop-blur-xl dark:border-border/60 dark:bg-popover/85 dark:ring-foreground/10 sm:max-w-none"
        style={{ width: "calc(100vw - 3rem)", maxWidth: "88rem", height: "calc(100dvh - 3rem)" }}
      >
        <header className="relative flex items-center border-b border-border/40 bg-background/50 px-2 backdrop-blur-xl dark:border-border/60 dark:bg-popover/80">
          <div data-tauri-drag-region="" className="flex-1 self-stretch" />
          <DialogTitle className="pointer-events-none absolute left-1/2 max-w-[45%] -translate-x-1/2 truncate text-xs font-medium text-muted-foreground" title={name}>
            {name}
          </DialogTitle>
          <div className="relative ml-auto flex shrink-0 items-center gap-0.5" aria-label={t("chat.imagePreview.zoomControls")}>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              disabled={zoom <= MIN_ZOOM}
              onClick={() => changeZoom(-1)}
              aria-label={t("chat.imagePreview.zoomOut")}
              className="size-10 rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:text-muted-foreground/25"
            >
              <IconMinus className="size-5" />
            </Button>
            <output
              aria-label={t("chat.imagePreview.zoomLevel")}
              className="flex h-10 min-w-14 items-center justify-center px-1 font-mono text-[11px] tabular-nums text-muted-foreground/65"
            >
              {zoomPercent}%
            </output>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              disabled={zoom >= MAX_ZOOM}
              onClick={() => changeZoom(1)}
              aria-label={t("chat.imagePreview.zoomIn")}
              className="size-10 rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:text-muted-foreground/25"
            >
              <IconPlus className="size-5" />
            </Button>
          </div>
          <DialogClose
            render={
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={t("chat.imagePreview.close")}
                className="ml-1 size-10 rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
              />
            }
          >
            <IconX className="size-4" />
          </DialogClose>
        </header>
        <DialogDescription className="sr-only">
          {t("chat.imagePreview.description")}
        </DialogDescription>
        <div
          ref={bindCanvas}
          tabIndex={0}
          onKeyDown={handleKeyDown}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={stopDragging}
          onPointerCancel={stopDragging}
          onLostPointerCapture={stopDragging}
          aria-label={t("chat.imagePreview.canvas", { name, zoom: zoomPercent })}
          className={`relative min-h-0 touch-none overflow-hidden bg-transparent outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${dragging ? "cursor-grabbing select-none" : "cursor-grab"}`}
        >
          <img
            data-slot="preview-image"
            src={src}
            alt={name}
            draggable={false}
            className="pointer-events-none absolute left-1/2 top-1/2 max-h-[calc(100%_-_4rem)] max-w-[calc(100%_-_4rem)] select-none object-contain drop-shadow-[0_12px_28px_rgba(0,0,0,0.2)] will-change-transform"
            style={{ transform: `translate(-50%, -50%) translate(${pan.x}px, ${pan.y}px) scale(${zoom})` }}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Prevents input devices from scaling the image beyond usable bounds. */
function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(zoom * 10) / 10));
}
