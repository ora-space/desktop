import { useTranslation } from "react-i18next";
import { appVersion } from "../../lib/app-version";
import { bundledReleaseNotes, type ReleaseNote } from "./bundled-release-notes";
import { ReleaseNoteList } from "./release-note-list";
import { visibleReleaseNotes } from "./release-notes-model";

/** Lists every bundled version up to the running one; it reads no files and no network. */
export function ReleaseNotesHistory({
  notes = bundledReleaseNotes,
  version = appVersion,
}: {
  notes?: readonly ReleaseNote[];
  version?: string;
}) {
  const { t } = useTranslation();
  const visible = visibleReleaseNotes(notes, version);
  return visible.length === 0 ? (
    <p className="text-sm text-muted-foreground">{t("releaseNotes.empty")}</p>
  ) : (
    <ReleaseNoteList notes={visible} />
  );
}
