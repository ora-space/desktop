import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import type { Locale } from "../../i18n/resource-bundle";
import {
  RELEASE_NOTE_CATEGORIES,
  type ReleaseNote,
} from "../../lib/release-note";

/** Renders notes as plain text in the interface language, grouped by category within each version. */
export function ReleaseNoteList({ notes }: { notes: readonly ReleaseNote[] }) {
  const { i18n, t } = useTranslation();
  const locale: Locale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  return (
    <div className="divide-y divide-border/70">
      {notes.map((note) => (
        <section
          key={note.version}
          aria-label={note.version}
          className="py-5 first:pt-0 last:pb-0"
        >
          <header className="flex items-baseline justify-between gap-3">
            <h3 className="text-sm font-semibold tabular-nums">
              {note.version}
            </h3>
            <time
              dateTime={note.date}
              className="text-xs text-muted-foreground tabular-nums"
            >
              {note.date}
            </time>
          </header>
          {RELEASE_NOTE_CATEGORIES.map((category) => {
            const entries = note.entries.filter(
              (entry) => entry.category === category,
            );
            if (entries.length === 0) return null;
            return (
              <div key={category} className="mt-3">
                <h4
                  className={cn(
                    "text-xs",
                    category === "important"
                      ? "font-semibold text-foreground"
                      : "text-muted-foreground",
                  )}
                >
                  {t(`releaseNotes.category.${category}`)}
                </h4>
                <ul className="mt-1 list-disc space-y-1 pl-4 marker:text-muted-foreground/60">
                  {entries.map((entry, index) => (
                    <li
                      key={index}
                      className="text-[13px] leading-[1.6] wrap-anywhere"
                    >
                      {entry[locale]}
                    </li>
                  ))}
                </ul>
              </div>
            );
          })}
        </section>
      ))}
    </div>
  );
}
