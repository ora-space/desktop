import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, cn } from "@ora/ui";
import { IconX } from "@tabler/icons-react";
import { OraMark } from "../../components/ora-mark";
import { appVersion } from "../../lib/app-version";
import { useOptionalPlatform } from "../../platform/use-platform";
import { bundledReleaseNotes, type ReleaseNote } from "./bundled-release-notes";
import { readPromptedVersion, writePromptedVersion } from "./prompted-version";
import { ReleaseNoteList } from "./release-note-list";
import {
  planReleaseNotesPrompt,
  type ReleaseNotesPrompt,
} from "./release-notes-model";

/** Shows the notes of every version installed since the last prompt, once, until dismissed. */
export function ReleaseNotesCard({
  align,
  notes = bundledReleaseNotes,
  version = appVersion,
}: {
  /** Lets the shell keep the card clear of a native surface, which paints above the DOM. */
  align: "left" | "right";
  notes?: readonly ReleaseNote[];
  version?: string;
}) {
  const { t } = useTranslation();
  const installation = useOptionalPlatform()?.installation;
  const [prompt, setPrompt] = useState<ReleaseNotesPrompt | null>(null);

  useEffect(() => {
    // Hosts without a local installation have no release to announce.
    if (installation === undefined) return;
    let active = true;
    void installation.isNew().then(
      (newInstallation) => {
        if (!active) return;
        const planned = planReleaseNotesPrompt({
          notes,
          appVersion: version,
          promptedVersion: readPromptedVersion(),
          newInstallation,
        });
        if (planned === null) return;
        writePromptedVersion(planned.version);
        if (planned.notes.length > 0) setPrompt(planned);
      },
      () => {
        // Without the answer a fresh installation could be prompted, so the next start decides.
      },
    );
    return () => {
      active = false;
    };
  }, [installation, notes, version]);

  if (prompt === null) return null;
  const title = t("releaseNotes.updated", { version: prompt.version });
  return (
    <aside
      aria-label={title}
      className={cn(
        "fixed z-40 flex max-h-[min(24rem,50dvh)] w-[22rem] max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-xl bg-popover text-popover-foreground shadow-lg ring-1 ring-foreground/10 duration-300 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-2",
        // Top right keeps the composer and its send button reachable while the card is open.
        align === "left" ? "bottom-4 left-4" : "right-4 top-16",
      )}
    >
      <header className="flex items-center gap-3 px-5 pb-3 pt-4">
        <OraMark className="shrink-0" />
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-sm font-semibold">{title}</h2>
          <p className="truncate text-xs text-muted-foreground">
            {t("releaseNotes.subtitle")}
          </p>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="-mr-2 size-7 shrink-0 self-start text-muted-foreground"
          aria-label={t("releaseNotes.dismiss")}
          title={t("releaseNotes.dismiss")}
          onClick={() => setPrompt(null)}
        >
          <IconX className="size-4" />
        </Button>
      </header>
      <div
        tabIndex={0}
        role="region"
        aria-label={title}
        className="min-h-0 overflow-y-auto overscroll-contain border-t [scrollbar-color:var(--border)_transparent] [scrollbar-width:thin] border-border/70 px-5 py-4 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      >
        <ReleaseNoteList notes={prompt.notes} />
      </div>
    </aside>
  );
}
