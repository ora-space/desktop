import { useEffect, useRef, type ReactNode } from "react";
import { Button, Spinner } from "@ora/ui";
import { IconExternalLink, IconRefresh, IconX } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { usePlatform } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { useSurfaceVisibility } from "./surface-occlusion";
import { useSurfaceBounds } from "./use-surface-bounds";

interface SurfaceHostProps {
  instance: number;
  /** Review-panel controls rendered in the header so expand/restore stay reachable. */
  toolbar?: ReactNode;
}

/**
 * The DOM placeholder for an embedded plugin surface.
 *
 * The native child view is positioned over the body area by `useSurfaceBounds`
 * and hidden whenever `useSurfaceVisibility` reports an overlay; the header
 * keeps reload/popout/close actions in DOM so they are never covered.
 */
export function SurfaceHost({ instance, toolbar }: SurfaceHostProps) {
  const { t } = useTranslation();
  const { surfaces } = usePlatform();
  const ref = useRef<HTMLDivElement>(null);
  const visible = useSurfaceVisibility(instance);
  const record = useSurfaceStore((s) => s.records[instance]);
  const failure = useSurfaceStore((s) => s.failures[instance]);

  // Visibility is pushed before bounds (declared below) so an unhidden view is
  // already showing when its fresh rectangle arrives.
  useEffect(() => {
    void surfaces.setVisible(instance, visible).catch(() => undefined);
  }, [instance, surfaces, visible]);
  // The layout keys the host by instance, so switching surfaces unmounts this host instead of
  // re-rendering it with `visible=false`. Without this unmount cleanup the native child view of
  // the previous instance keeps painting over the new one. Kept separate from the sync effect
  // above so overlay toggles do not issue a redundant hide before every show.
  useEffect(
    () => () => {
      void surfaces.setVisible(instance, false).catch(() => undefined);
    },
    [instance, surfaces],
  );
  useSurfaceBounds(ref, instance, visible);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-1 border-b border-border px-2">
        <span className="truncate text-sm font-medium">{record?.title}</span>
        <div className="flex-1" />
        {toolbar}
        <Button
          size="icon-sm"
          variant="ghost"
          className="size-7"
          aria-label={t("surface.reload")}
          title={t("surface.reload")}
          onClick={() => void surfaces.reload(instance)}
        >
          <IconRefresh />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          className="size-7"
          aria-label={t("surface.popout")}
          title={t("surface.popout")}
          onClick={() => void surfaces.popout(instance)}
        >
          <IconExternalLink />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          className="size-7"
          aria-label={t("surface.close")}
          title={t("surface.close")}
          onClick={() => void surfaces.close(instance)}
        >
          <IconX />
        </Button>
      </header>
      <div
        ref={ref}
        data-testid="surface-placeholder"
        className="relative flex min-h-0 flex-1 items-center justify-center bg-muted/20"
      >
        {record?.state === "failed" && (
          <div className="flex flex-col items-center gap-3 p-6 text-center text-sm text-muted-foreground">
            <p>{t("surface.failed", { reason: failure ?? "" })}</p>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void surfaces.reload(instance)}
            >
              {t("surface.retry")}
            </Button>
          </div>
        )}
        {record?.state === "opening" && <Spinner />}
      </div>
    </div>
  );
}
