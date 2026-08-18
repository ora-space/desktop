# lib

Small, domain-agnostic helpers used by app-shell features. This directory must
not grow into a feature module: chat, diff, and files keep their own folders.

## Responsibilities

- Time and label formatting (`format.ts`)
- Avatar color derivation (`avatar.ts`)
- Shared TypeScript aliases (`types.ts`)
- Review-panel motion helpers (`panel-motion.ts`)
- Workspace path matching shared by Changes and chat inline links (`workspace-path.ts`)

## Path matching

`normalizeDiffPath` and `pathsMatchForWorkspace` are the single source of truth
for comparing ACP / Git / chat path tokens. Chat links and the Changes panel
must import these helpers instead of copying slash-normalization locally.

Absolute ACP paths are converted to workspace-relative form with
`stripTaskCwdPrefix` before `readWorkspaceFile` or Diff requests. Chat
classification and the Files panel both call this helper so a late Files
open of an absolute tool path does not hit the backend’s rooted-path
rejection. OS handoff uses `joinOsAbsolutePath` so Explorer, VS Code, and
Copy path receive a host path rather than an internal URI.
