# Release notes

English | [中文](release-notes.zh.md)

Each stable version has one handwritten file, `release-notes/<version>.json`, in this repository. It is
the only source of that version's notes. `scripts/release-notes.ts` owns the format and its checks.

## Writing notes

Before a release, describe the user-visible changes between the previous stable tag and the commit to be
packaged. A local AI tool may draft the text; a person edits it and opens a normal pull request. Before
tagging, review the commits merged after that pull request, add what is missing, and tag the reviewed
commit.

```json
{
  "schema": 1,
  "version": "0.7.0",
  "date": "2026-12-10",
  "entries": [{ "category": "feature", "zh-CN": "……", "en-US": "…" }]
}
```

| Field                                | Rule                                                                                                                                             |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `schema`                             | `1`                                                                                                                                              |
| `version`                            | A stable version without `v`, equal to the file name                                                                                             |
| `date`                               | The planned release date, `YYYY-MM-DD`; it may differ from the actual one                                                                        |
| `entries`                            | 1 to 50 entries                                                                                                                                  |
| `entries[].category`                 | `important`, `feature`, `improvement`, or `fix`                                                                                                  |
| `entries[].zh-CN`, `entries[].en-US` | Required plain text on one line, at most 500 characters, without control, line separator, bidirectional formatting, or lone surrogate characters |

No other fields are accepted; notes do not list source pull requests or commit ranges.

## Checks

The tooling tests validate every notes file in the repository, including in pull requests that only change
`release-notes/`.

`task package:desktop -- --tag <version>` and the release workflow build nothing unless:

- `release-notes/` has the notes of the stable version. `v0.7.0`, `v0.7.0-rc.1`, and `0.7.0+suffix` all
  need `0.7.0.json`.
- Every notes file is valid, not only the one for this version.
- `release-notes/` matches the checked-out commit: no modified, deleted, untracked, or ignored files, and
  the sources are a Git checkout. Edits elsewhere do not matter.

Packaging without `--tag` skips these checks.

## Display

The Desktop frontend build injects every notes file of its checkout; the app reads no files and no
network to show them. An untagged build with invalid notes hides all of them instead of failing. Texts
are always rendered as plain text, in the interface language, newest version first, and never for a
version newer than the running stable version (`0.7.0-rc.1` shows up to `0.7.0`).

- **Settings → Release notes** lists the history.
- **A dismissible card** appears once after an update. It lists every version above the highest one
  already prompted on this machine, then remembers the running version. A downgrade, a build without
  notes of its own version, and a fresh installation show no card; only the fresh installation
  remembers the version.

Without a record, `is_new_installation` separates a fresh installation from an upgrade: it is true when
this process opened a database that had no applied migration. An upgrade without a record lists every
applicable version.

This startup-origin flag is independent of the durable first-install data journal. If a first
launch commits migrations and then stops before data initialization, restarting resumes the data
but reports an existing database. Without a prior display record, release notes can appear once.

The card only lists versions above v0.6.0, the last release without this feature, and a build of v0.6.0 or
earlier neither shows it nor remembers its version. The notes of v0.3.0 to v0.6.0 were written afterwards
for the history: their `date` is when the release was published, and v0.1.0 and earlier have none.

### Before an update

`latest.json` carries the notes of the latest ten versions up to the released one in
`ora_release_notes: { schema, versions }`, beside the standard `notes` text, which must stay a string for the
updater plugin. Only known fields are written. The release job fails when the released version has no
notes or any file is invalid.

The manifest is not signed and every installed client reads it, so Desktop treats the notes as an
untrusted contract, apart from the update check. Unknown fields are ignored; an unknown schema, a broken
content rule, more than ten versions or more than 50 entries hides all notes and logs the reason. The
check, download and installation do not change. Within one schema, the limits written here may never
exceed those of a released client.

Settings → Release notes shows the offered release above the history, with the notes above the running
stable version up to the offered one, and the install action, which still asks for one confirmation because
installing restarts the app. Manifest notes never enter the history and are not stored. An installation that must be updated
manually sees the notes without that action.
