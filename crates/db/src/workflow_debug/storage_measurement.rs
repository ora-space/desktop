use super::*;
use pretty_assertions::assert_eq;

/// Measures allocated SQLite pages with the same growing ledger, with and without recording
/// debug evidence. This is a manual sizing fixture, not a platform-dependent CI budget.
#[test]
#[ignore = "manual storage measurement"]
fn foreach_storage_growth() {
    for width in [32, 1024, 4096, 10240, 32768] {
        for evidence in [false, true] {
            let (_temp, _, store) = fixture();
            store.pool.with_connection(|db| {
                if !evidence { db.execute_batch("DROP TRIGGER workflow_debug_admit; DROP TRIGGER workflow_debug_node_insert; DROP TRIGGER workflow_debug_node_change;")?; }
                let size = || db.query_row("SELECT page_count*page_size FROM pragma_page_count, pragma_page_size", [], |r| r.get::<_, i64>(0));
                let before = size()?;
                let items: Vec<_> = (0..100).map(|i|format!("{i:03}")).collect();
                let mut ledger = serde_json::Map::new();
                let output = "x".repeat(width);
                bind_graph(db, json!({"nodes":[{"id":"agent","parentId":"iter"}],"edges":[]}))?;
                db.execute("UPDATE workflow_runs SET input=?1 WHERE id='other'", [json!({"items":items}).to_string()])?;
                insert_attempt(db,"container","iter","iteration",None)?;
                for (i, item) in items.iter().enumerate() {
                    let pool = json!({"catalog":{},"values":{"start.items":items,"iter.index":i,"iter.item":item},"revision":i});
                    let payload = json!({"variablePool":pool,"iterationLedger":{"iter":ledger}});
                    db.execute("UPDATE workflow_runs SET payload=?1 WHERE id='other'", [payload.to_string()])?;
                    let id = format!("agent-{i}");
                    insert_attempt(db,&id,"agent","agent",Some(i as u32))?;
                    if evidence {
                        let restored: String = db.query_row("SELECT json_extract(entry_state,'$.runPayload') FROM workflow_debug_entries WHERE node_run_id=?1",[&id], |row| row.get(0))?;
                        assert_eq!(serde_json::from_str::<Value>(&restored)?, payload);
                    }
                    if evidence { db.execute("UPDATE workflow_debug_attempts SET dispatch_input=?2 WHERE node_run_id=?1",params![id,json!({"variablePool":pool,"prompt":item}).to_string()])?; }
                    db.execute("UPDATE workflow_node_runs SET status=2,output=?2 WHERE id=?1",params![id,output])?;
                    ledger.insert(i.to_string(),json!({"status":"succeeded","item":item,"output":output}));
                }
                println!("output_bytes={width} evidence={evidence} allocated_growth={}",size()? - before);
                Ok(())
            }).unwrap();
        }
    }
}

/// An overwrite of the same logical item must preserve each member's historical payload on reopen.
#[test]
fn reopened_members_keep_their_own_ledger_and_pool() {
    let (_temp, location, store) = fixture();
    let first = json!({"iterationLedger":{"loop.x":{"0":{"output":"old"}},"empty":{}},"variablePool":{"values":{"x":"first"}}});
    let second = json!({"iterationLedger":{"loop.x":{"0":{"output":"new"}},"empty":{}},"variablePool":{"values":{"x":"second"}}});
    store
        .pool
        .with_connection(|db| {
            for (id, kind, payload) in [("a", "agent", &first), ("b", "condition", &second)] {
                db.execute(
                    "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                    [payload.to_string()],
                )?;
                insert_attempt(db, id, kind, kind, Some(0))?;
            }
            Ok(())
        })
        .unwrap();
    drop(store);
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(&location, &default_migration_catalog().unwrap())
        .unwrap();
    pool.with_connection(|db| {
        for (id,expected) in [("a",first),("b",second)] {
            let raw: String = db.query_row("SELECT json_extract(entry_state,'$.runPayload') FROM workflow_debug_entries WHERE node_run_id=?1",[id],|row|row.get(0))?;
            assert_eq!(serde_json::from_str::<Value>(&raw)?,expected);
        }
        Ok(())
    }).unwrap();
}
