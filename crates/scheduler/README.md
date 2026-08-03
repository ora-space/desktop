# ora-scheduler

`ora-scheduler` runs recurring asynchronous jobs on cron schedules within an existing Tokio runtime. It provides a small lifecycle boundary for registering jobs, validating their schedules, starting them, and stopping them cooperatively.

## Public model

- `Job` defines a stable name, a cron expression, and the asynchronous work performed on each tick.
- `Scheduler` collects heterogeneous jobs, owns an explicitly supplied IANA timezone, and starts one independent Tokio task for each registered job.
- `SchedulerError` reports invalid cron expressions before any job task is spawned.
- `BoxFuture` is the object-safe return type used by `Job::run`.

## Lifecycle and scheduling

`Scheduler::new` requires a `chrono_tz::Tz`, making the timezone choice explicit at the composition root. `Scheduler::start` parses every registered cron expression before spawning work, so invalid configuration fails atomically at startup. Valid jobs calculate their fire times in that configured timezone and execute sequentially within their own task. A slow invocation therefore does not overlap with the next invocation of the same job, while different jobs can run concurrently.

The caller owns shutdown through a `CancellationToken`. Cancellation stops jobs while they are waiting for their next tick, and the returned `JoinHandle` completes after all job tasks have exited. Job panics and task join failures are not exposed through the scheduler's error type; operational visibility is provided through structured Ora logging.

## Boundaries

The crate owns in-process timing and cooperative shutdown only. It does not persist schedules, provide distributed coordination, retry failed work, enforce execution timeouts, or record job outcomes. Job implementations and their composition roots own those policies and any required dependencies.
