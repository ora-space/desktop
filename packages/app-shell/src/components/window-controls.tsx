import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { usePlatform } from "@ora/platform";
import { cn } from "@ora/ui";

/**
 * Custom caption buttons for a frameless desktop window (minimize / maximize /
 * close), painted only where the platform asks the app to own its chrome -
 * Windows and Linux. The Web host and macOS report `windowControls.kind ===
 * "none"`, so this renders nothing and the surrounding header collapses back to
 * its native layout.
 *
 * The glyphs are hand-drawn at a 10px cap height so they read crisply at the
 * app's density, and the controls lean on the same neutral tokens as the rest of
 * the toolbar - a muted rest state, a soft hover, and the one convention worth
 * keeping: a red wash under the close button.
 */
export function WindowControls() {
  const { windowControls } = usePlatform();
  const { t } = useTranslation();
  const [maximized, setMaximized] = useState(false);

  const overlay = windowControls.kind === "overlay" ? windowControls : null;

  useEffect(() => {
    if (overlay === null) return;
    let active = true;
    void overlay.isMaximized().then((value) => {
      if (active) setMaximized(value);
    });
    const unsubscribe = overlay.subscribeMaximized((value) => setMaximized(value));
    return () => {
      active = false;
      unsubscribe();
    };
  }, [overlay]);

  if (overlay === null) return null;

  return (
    <div className="flex items-center gap-0.5 pl-1" role="group" aria-label={t("window.controls")}>
      <CaptionButton label={t("window.minimize")} onClick={() => void overlay.minimize()}>
        <MinimizeGlyph />
      </CaptionButton>
      <CaptionButton
        label={maximized ? t("window.restore") : t("window.maximize")}
        onClick={() => void overlay.toggleMaximize()}
      >
        {maximized ? <RestoreGlyph /> : <MaximizeGlyph />}
      </CaptionButton>
      <CaptionButton label={t("window.close")} tone="close" onClick={() => void overlay.close()}>
        <CloseGlyph />
      </CaptionButton>
    </div>
  );
}

function CaptionButton({
  label,
  onClick,
  tone = "default",
  children,
}: {
  label: string;
  onClick: () => void;
  tone?: "default" | "close";
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "flex size-8 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
        tone === "close"
          ? "hover:bg-red-500 hover:text-white"
          : "hover:bg-muted hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

/** Shared frame for the 10x10 caption glyphs so every stroke lines up. */
function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth={1}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

function MinimizeGlyph() {
  return (
    <Glyph>
      <line x1="1.5" y1="5" x2="8.5" y2="5" strokeLinecap="round" />
    </Glyph>
  );
}

function MaximizeGlyph() {
  return (
    <Glyph>
      <rect x="1.5" y="1.5" width="7" height="7" rx="1.5" />
    </Glyph>
  );
}

function RestoreGlyph() {
  return (
    <Glyph>
      <rect x="1.3" y="2.9" width="5.5" height="5.5" rx="1.3" />
      <path d="M3.5 2.9V2.6A1.3 1.3 0 0 1 4.8 1.3h2.9A1.3 1.3 0 0 1 9 2.6v2.9a1.3 1.3 0 0 1-1.3 1.3h-.3" />
    </Glyph>
  );
}

function CloseGlyph() {
  return (
    <Glyph>
      <line x1="1.8" y1="1.8" x2="8.2" y2="8.2" strokeLinecap="round" />
      <line x1="8.2" y1="1.8" x2="1.8" y2="8.2" strokeLinecap="round" />
    </Glyph>
  );
}
