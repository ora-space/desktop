# ora-surface::state

Pure state machine of one surface instance. Decides; never executes.

## Responsibilities

- Model the lifecycle as `SurfaceState` (`Opening`, `Embedded`, `Windowed`, `Migrating`,
  `Closing`, `Failed`). There is no `Closed` variant: a transition whose `next` is `None` ends
  the instance and the registry removes it.
- `apply_command` handles user/host intents (`Close`, `Popout`, `Dock`, `SetVisible`, `Rebuild`).
- `apply_completion` handles the end of asynchronous host operations (`Opened`, `Migrated`,
  `Closed`), each carrying the `OperationId` ticket the transition that started it handed out.
- Return `SurfaceEffect`s (`CreateWebview`, `Reparent`, `DestroyWebview`,
  `SetNativeVisibility`, `Emit(SurfaceEvent)`) for the host to execute outside any lock.

## Invariants

- Functions are pure and total over `(state, input)`; refused inputs return `TransitionError`
  (`Busy`, `InvalidForState`) or `StaleCompletion` without touching the state.
- A completion whose ticket does not match the pending operation is stale, whatever the state.
- `Close` during `Opening`/`Migrating` is remembered (`close_requested`) and honored as soon as
  the pending operation completes; no `Opened`/`Migrated` event is emitted in that case.
- `Close` during `Closing` is idempotent; `Close` on `Failed` ends the instance immediately
  because no webview exists.
- `Rebuild` bumps `ViewGeneration` so hosts can discard callbacks from the destroyed page.
- `next_operation` is only invoked by transitions that start a new asynchronous operation.

## Transition table

Rows are current states, columns are inputs. "stale" means `StaleCompletion`.

| State     | Close              | Popout               | Dock                 | SetVisible                     | Rebuild                            | Opened(ok)                                                    | Opened(err)                                                   | Migrated(ok)                                                        | Migrated(err)                                                                   | Closed             |
| --------- | ------------------ | -------------------- | -------------------- | ------------------------------ | ---------------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------ |
| Opening   | remember           | Busy                 | Busy                 | Invalid                        | Invalid                            | mounted + Emit(Opened); Closing + Destroy if close remembered | Failed + Emit(Failed); end + Emit(Closed) if close remembered | stale                                                               | stale                                                                           | stale              |
| Embedded  | Closing + Destroy  | Migrating + Reparent | Invalid              | Embedded + SetNativeVisibility | Opening + Destroy + Create, view+1 | stale                                                         | stale                                                         | stale                                                               | stale                                                                           | stale              |
| Windowed  | Closing + Destroy  | Invalid              | Migrating + Reparent | Invalid                        | Opening + Destroy + Create, view+1 | stale                                                         | stale                                                         | stale                                                               | stale                                                                           | stale              |
| Migrating | remember           | Busy                 | Busy                 | Invalid                        | Invalid                            | stale                                                         | stale                                                         | mounted(to) + Emit(Migrated); Closing + Destroy if close remembered | mounted(from) + Emit(MigrateFailed); plus Closing + Destroy if close remembered | stale              |
| Closing   | idempotent         | Invalid              | Invalid              | Invalid                        | Invalid                            | stale                                                         | stale                                                         | stale                                                               | stale                                                                           | end + Emit(Closed) |
| Failed    | end + Emit(Closed) | Invalid              | Invalid              | Invalid                        | Opening + Create                   | stale                                                         | stale                                                         | stale                                                               | stale                                                                           | stale              |

Every cell is covered by the table-driven tests in `tests.rs`.
