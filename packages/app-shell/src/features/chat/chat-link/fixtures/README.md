# chat-link recorded fixtures

Recorded ACP transcripts replayed by `recorded-sessions.test.tsx`. Each file is a
JSON array of session-log lines from one real agent turn, so the tool payloads keep the shape Ora actually receives:
`content` and `rawOutput` carrying the same text, `locations` naming the cwd,
ANSI styling intact. Hand-written fixtures kept missing exactly those details,
and both chat-link link bugs shipped through that gap.

## Adding a case

When a path in chat does not link (or links to the wrong surface):

1. Find the session log under the app data directory
   (`ORA_DATA_DIR/sessions/<xx>/<yy>/<sessionId>.jsonl`; the desktop dev task
   points `ORA_DATA_DIR` at the repo `.data` directory).
2. Copy the records for that turn into a new `.json` array here: the
   `user_message_chunk`, the `tool_call` (verbatim — that payload is the
   subject), the `agent_message_chunk`, and the closing `turnEnded`.
3. Import it in `recorded-sessions.test.tsx` and add one `RECORDED_CASES` entry
   listing the tokens that must link and the surface each one opens.

Prose text may be rewritten for readability (agent output on a GBK console is
recorded mojibake); tool payloads must stay byte-for-byte what the agent sent.
