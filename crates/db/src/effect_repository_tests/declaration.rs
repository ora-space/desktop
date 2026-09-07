use super::{declaration, fixture};
use crate::SqliteEffectRepository;
use pretty_assertions::assert_eq;
use rusqlite::params;

/// Replays the commit order of agent startup racing a worker that sampled its clock later.
#[test]
fn unchanged_declarations_preserve_consumer_timestamps_when_committed_out_of_order()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, pool, workspace) = fixture();
    let clock = crate::test_clock::TestClock::new(30);
    let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());

    // Startup can race before any Workspace exists, or after Targets have already been paired.
    for (identity, workspaces) in [
        ("official/no-workspace", &[][..]),
        ("official/with-workspace", std::slice::from_ref(&workspace)),
    ] {
        clock.set(20);
        let consumer = declaration(identity);
        let revision = repository.declare_consumer(&consumer, workspaces)?;
        for timestamp in [30, 10, 25] {
            clock.set(timestamp);
            assert_eq!(
                repository.declare_consumer(&consumer, workspaces,)?,
                revision,
            );
        }

        let stored = pool.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT current_revision_id, lifecycle, created_at, updated_at,
                            (SELECT COUNT(*) FROM effect_consumer_revisions WHERE consumer_id = ?1)
                     FROM effect_consumers WHERE id = ?1",
                    params![consumer.consumer.storage_key()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })?;
        assert_eq!(
            stored,
            (
                revision.as_str().to_string(),
                "declared".to_string(),
                20,
                30,
                1
            ),
        );
    }
    Ok(())
}
