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

| Field                                | Rule                                                                                                                             |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| `schema`                             | `1`                                                                                                                              |
| `version`                            | A stable version without `v`, equal to the file name                                                                             |
| `date`                               | The planned release date, `YYYY-MM-DD`; it may differ from the actual one                                                        |
| `entries`                            | At least one entry                                                                                                               |
| `entries[].category`                 | `important`, `feature`, `improvement`, or `fix`                                                                                  |
| `entries[].zh-CN`, `entries[].en-US` | Required plain text on one line, at most 500 characters, without control, line separator, or bidirectional formatting characters |

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
