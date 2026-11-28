use super::*;
use pretty_assertions::assert_eq;

/// Measures allocated SQLite pages with the same growing ledger, with and without recording
/// debug evidence. This is a manual sizing fixture, not a platform-dependent CI budget.
#[test]
#[ignore = "manual storage measurement"]
fn foreach_storage_growth() {
    for (shape, width) in [
        ("output", 32),
        ("output", 1024),
        ("output", 4096),
        ("output", 10240),
        ("output", 32768),
        ("large-input", 32),
        ("condition-large-input", 32),
        ("chained", 32),
        ("upstream", 32),
    ] {
        for evidence in [false, true] {
            measure_foreach(
                &default_migration_catalog().unwrap(),
                shape,
                width,
                evidence,
            );
        }
    }
}

/// Compares the actual write statements on the same fixture with both versions of the codec.
#[test]
#[ignore = "manual timing measurement"]
fn foreach_evidence_write_cost() {
    for version in ["0028", "0029"] {
        let catalog = catalog_through(version);
        for (shape, width) in [("output", 32768), ("large-input", 32)] {
            measure_foreach(&catalog, shape, width, /*evidence*/ true);
        }
    }
}

/// Builds a historical catalog so later migrations cannot change the comparison.
fn catalog_through(version: &str) -> crate::MigrationCatalog {
    let catalog = default_migration_catalog().unwrap();
    crate::MigrationCatalog::new(
        catalog
            .target_versions()
            .iter()
            .copied()
            .take_while(|v| *v <= version)
            .map(|v| catalog.migration(v).unwrap().clone())
            .collect(),
    )
    .unwrap()
}

/// Excludes fixture serialization and history reconstruction from admission and dispatch timings.
fn measure_foreach(catalog: &crate::MigrationCatalog, shape: &str, width: usize, evidence: bool) {
    let (_temp, _, store) = fixture_with_catalog(catalog);
    let version = catalog.target_versions().last().unwrap();
    store.pool.with_connection(|db| {
        if !evidence { db.execute_batch("DROP TRIGGER workflow_debug_admit; DROP TRIGGER workflow_debug_node_insert; DROP TRIGGER workflow_debug_node_change;")?; }
        let size = || db.query_row("SELECT page_count*page_size FROM pragma_page_count, pragma_page_size", [], |r| r.get::<_, i64>(0));
        // Migration 0025 frees pages. Start both measurements without that reserve.
        db.execute_batch("VACUUM")?;
        let before = size()?;
        let items: Vec<_> = (0..100).map(|i| if matches!(shape,"large-input" | "condition-large-input") { format!("{i:03}{}", "x".repeat(10237)) } else { format!("{i:03}") }).collect();
        let mut ledger = serde_json::Map::new();
        let output = "x".repeat(width);
        let kind = if shape == "condition-large-input" { "condition" } else { "agent" };
        bind_graph(db, json!({"nodes":[{"id":kind,"parentId":"iter"}],"edges":[]}))?;
        db.execute("UPDATE workflow_runs SET input=?1 WHERE id='other'", [json!({"items":items}).to_string()])?;
        insert_attempt(db,"container","iter","iteration",None)?;
        let mut admission = std::time::Duration::ZERO;
        let mut dispatch = std::time::Duration::ZERO;
        for (i, item) in items.iter().enumerate() {
            let mut pool = json!({"catalog":{},"values":{"start.items":items,"iter.index":i,"iter.item":item},"revision":i});
            if shape == "chained" {
                let prior: Vec<_> = (0..100).map(|i| format!("{i:03}{}", "p".repeat(10237))).collect();
                pool["values"]["prior.output"] = json!(prior);
                pool["values"]["prior.entries"] = json!(prior.iter().enumerate().map(|(index,output)| json!({"index":index,"output":output})).collect::<Vec<_>>());
            } else if shape == "upstream" {
                pool["values"]["upstream.output"] = json!("u".repeat(102400));
            }
            let payload = json!({"variablePool":pool,"iterationLedger":{"iter":ledger}});
            db.execute("UPDATE workflow_runs SET payload=?1 WHERE id='other'", [payload.to_string()])?;
            let id = format!("{kind}-{i}");
            let start = std::time::Instant::now();
            insert_attempt(db,&id,kind,kind,Some(i as u32))?;
            admission += start.elapsed();
            if evidence {
                let restored: String = db.query_row("SELECT json_extract(entry_state,'$.runPayload') FROM workflow_debug_entries WHERE node_run_id=?1",[&id], |row| row.get(0))?;
                assert_eq!(serde_json::from_str::<Value>(&restored)?, payload);
            }
            if kind == "condition" {
                if evidence {
                    record_condition_pool(db,"other",&id,pool)?;
                    db.execute("UPDATE workflow_node_runs SET payload=?2 WHERE id=?1",params![id,json!({"debugCondition":{"branch":"A"}}).to_string()])?;
                }
            } else if evidence {
                let body = json!({"variablePool":pool,"prompt":item}).to_string();
                let start = std::time::Instant::now();
                db.execute("UPDATE workflow_debug_attempts SET dispatch_input=?2 WHERE node_run_id=?1",params![id,body])?;
                dispatch += start.elapsed();
            }
            db.execute("UPDATE workflow_node_runs SET status=2,output=?2 WHERE id=?1",params![id,(kind == "agent").then_some(&output)])?;
            ledger.insert(i.to_string(),json!({"status":"succeeded","item":item,"output":output}));
        }
        println!("schema={version} shape={shape} output_bytes={width} evidence={evidence} admission_ms={} dispatch_ms={} allocated_growth={}",admission.as_millis(),dispatch.as_millis(),size()? - before);
        Ok(())
    }).unwrap();
}

/// Both codecs must restore attempts captured before and after an upgrade, even on rollback.
#[test]
fn reopened_members_keep_their_own_ledger_and_pool() {
    let (_temp, location, store) = fixture_with_catalog(&catalog_through("0028"));
    drop(store);
    let first = json!({"iterationLedger":{"loop.x":{"0":{"output":"old"}},"empty":{}},"variablePool":{"values":{"x":"first"}}});
    let second = json!({"iterationLedger":{"loop.x":{"0":{"output":"new"}},"empty":{}},"variablePool":{"values":{"x":"second"}}});
    for (version, id, kind, payload) in [
        ("0028", "a", "agent", &first),
        ("0029", "b", "condition", &second),
        ("0028", "c", "loop", &first),
        ("0029", "d", "iteration", &second),
    ] {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&location, &catalog_through(version))
            .unwrap();
        pool.with_connection(|db| {
            db.execute(
                "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                [payload.to_string()],
            )?;
            insert_attempt(db, id, kind, kind, Some(0))?;
            Ok(())
        })
        .unwrap();
    }
    for version in ["0029", "0028", "0029"] {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&location, &catalog_through(version))
            .unwrap();
        pool.with_connection(|db| {
            for (id,expected) in [("a",&first),("b",&second),("c",&first),("d",&second)] {
                let compact: String = db.query_row("SELECT body FROM workflow_debug_documents WHERE node_run_id=?1 AND kind='run'",[id],|row|row.get(0))?;
                let compact: Value = serde_json::from_str(&compact)?;
                assert!(compact.get("iterationLedger").is_none());
                assert!(compact["variablePool"].get("values").is_none());
                let raw: String = db.query_row("SELECT json_extract(entry_state,'$.runPayload') FROM workflow_debug_entries WHERE node_run_id=?1",[id],|row|row.get(0))?;
                assert_eq!(serde_json::from_str::<Value>(&raw)?,*expected);
            }
            Ok(())
        }).unwrap();
    }
}

/// Entry and dispatch reuse the same pool value. Changing input cannot rewrite an old attempt.
#[test]
fn immutable_values_are_shared_across_admission_and_dispatch() {
    let (_temp, _, store) = fixture();
    let pool =
        json!({"values":{"start.items":[1,null,false,{"k":"v"}], "a.b":"text", "empty":null}});
    store
        .pool
        .with_connection(|db| {
            for (id, input) in [("one", "first"), ("two", "first"), ("three", "changed")] {
                db.execute(
                    "UPDATE workflow_runs SET input=?1,payload=?2 WHERE id='other'",
                    params![input, json!({"variablePool":pool}).to_string()],
                )?;
                insert_attempt(db, id, id, "agent", None)?;
            }
            Ok(())
        })
        .unwrap();
    for id in ["one", "two", "three"] {
        assert!(
            store
                .record_agent_dispatch("other", id, &json!({"variablePool":pool,"prompt":id}))
                .unwrap()
        );
    }
    store.pool.with_connection(|db| {
        let count: i64 = db.query_row("SELECT count(*) FROM workflow_debug_outcomes WHERE run_id='other'",[],|r|r.get(0))?;
        assert_eq!(count,5); // Two input versions and three distinct pool values.
        for (id,input) in [("one","first"),("two","first"),("three","changed")] {
            let packed: bool = db.query_row("SELECT a.dispatch_input='{}' AND json_type(d.body,'$.variablePool.values') IS NULL AND d.refs <> '{}' FROM workflow_debug_attempts a JOIN workflow_debug_documents d USING(node_run_id) WHERE a.node_run_id=?1 AND d.kind='dispatch'",[id],|r|r.get(0))?;
            assert!(packed);
            let (entry,dispatch):(String,String)=db.query_row("SELECT entry_state,dispatch_input FROM workflow_debug_entries WHERE node_run_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let entry:Value=serde_json::from_str(&entry)?;
            assert_eq!(entry["runInput"],input);
            let payload:Value=serde_json::from_str(entry["runPayload"].as_str().unwrap())?;
            assert_eq!(payload["variablePool"],pool);
            assert_eq!(serde_json::from_str::<Value>(&dispatch)?,json!({"variablePool":pool,"prompt":id}));
        }
        Ok(())
    }).unwrap();
}

#[test]
fn admission_rolls_back_when_the_input_digest_matches_different_bytes() {
    let (_temp, _, store) = fixture();
    store.pool.with_connection(|db| {
        db.execute("UPDATE workflow_runs SET input='new input',payload=?1 WHERE id='other'",
            [json!({"iterationLedger":{"iter":{"0":{"output":"before collision"}}}}).to_string()])?;
        db.execute("INSERT INTO workflow_debug_outcomes(run_id,outcome,digest) VALUES('other','different bytes',ora_debug_hash('new input'))",[])?;
        let error = insert_attempt(db,"collision","agent","agent",None).unwrap_err();
        assert!(matches!(error, DatabaseError::Sqlite(rusqlite::Error::SqliteFailure(code,Some(message)))
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER && message == "workflow debug content hash collision"));
        let count: i64 = db.query_row("SELECT count(*) FROM workflow_node_runs WHERE id='collision'",[],|r|r.get(0))?;
        assert_eq!(count,0);
        let remaining: (i64, i64) = db.query_row(
            "SELECT (SELECT count(*) FROM workflow_debug_pending_refs),
                    (SELECT count(*) FROM workflow_debug_outcomes WHERE run_id='other')",
            [], |row| Ok((row.get(0)?, row.get(1)?)))?;
        assert_eq!(remaining, (0, 1));
        Ok(())
    }).unwrap();
}

#[test]
fn loop_round_document_restores_interned_values() {
    let (_temp, _, store) = fixture();
    store.pool.with_connection(|db| {
        insert_attempt(db,"lp","loop","loop",None)?;
        let state = json!({"variablePool":{"values":{"loop.draft":"third"}}});
        db.execute("INSERT INTO workflow_execution_scopes(id,run_id,parent_loop_node_run_id,round_index,status,state,created_at,updated_at)
            VALUES('round3','other','lp',3,1,?1,1,1)",[state.to_string()])?;
        db.execute("INSERT INTO workflow_node_runs(id,run_id,scope_id,node_id,node_type,status,created_at,updated_at)
            VALUES('writer3','other','round3','writer','agent',1,1,1)",[])?;
        let packed: bool = db.query_row("SELECT json_type(body,'$.variablePool.values') IS NULL AND refs <> '{}' FROM workflow_debug_documents WHERE node_run_id='writer3' AND kind='scope'",[],|r|r.get(0))?;
        assert!(packed);
        let (raw,restored):(String,String) = db.query_row("SELECT a.entry_state,e.entry_state FROM workflow_debug_attempts a JOIN workflow_debug_entries e USING(node_run_id) WHERE a.node_run_id='writer3'",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
        assert!(serde_json::from_str::<Value>(&raw)?.get("scopeState").is_none());
        let envelope: Value = serde_json::from_str(&restored)?;
        assert_eq!(serde_json::from_str::<Value>(envelope["scopeState"].as_str().unwrap())?,state);
        Ok(())
    }).unwrap();
}

/// Counts digest calls at the SQLite boundary and checks that equal bytes in two runs
/// receive separate IDs. Repeated captures within one run must keep the original ID.
#[test]
fn each_capture_hashes_each_value_once_and_binds_it_to_its_run() {
    use rusqlite::functions::FunctionFlags;
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex};

    let (_temp, _, store) = fixture();
    store.pool.with_connection(|db| {
        let hashed = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::clone(&hashed);
        db.create_scalar_function("ora_debug_hash", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
            let value: String = ctx.get(0)?;
            let digest = Sha256::digest(value.as_bytes()).to_vec();
            calls.lock().unwrap().push(value);
            Ok(digest)
        })?;
        let outcome = json!({"output": "old"});
        let pool = json!({"values": {"field.with.dots": "shared"}});
        let payload = json!({"variablePool": pool, "iterationLedger": {"it": {"0": outcome}, "empty": {}}});
        db.execute(
            "UPDATE workflow_runs SET input='raw input',payload=?1",
            [payload.to_string()],
        )?;
        for (run, attempt) in [("F", "capture-f"), ("other", "capture-other")] {
            db.execute(
                "INSERT INTO workflow_node_runs(id,run_id,scope_id,node_id,node_type,status,created_at,updated_at)
                 SELECT ?1,run_id,scope_id,?1,'agent',1,1,1 FROM workflow_run_root_scopes WHERE run_id=?2",
                params![attempt, run],
            )?;
            let mut actual = std::mem::take(&mut *hashed.lock().unwrap());
            actual.sort();
            let mut expected = [json!("shared").to_string(), "raw input".to_owned(), outcome.to_string()];
            expected.sort();
            assert_eq!(actual, expected.to_vec());

            db.execute(
                "UPDATE workflow_debug_attempts SET dispatch_input=?1 WHERE node_run_id=?2",
                params![json!({"variablePool": pool}).to_string(), attempt],
            )?;
            assert_eq!(std::mem::take(&mut *hashed.lock().unwrap()), vec![json!("shared").to_string()]);
        }
        let ids: i64 = db.query_row(
            "SELECT count(DISTINCT o.id) FROM workflow_debug_documents d
             JOIN workflow_node_runs n ON n.id=d.node_run_id
             JOIN json_each(d.refs) ref
             JOIN workflow_debug_outcomes o ON o.id=ref.value AND o.run_id=n.run_id
             WHERE n.id IN ('capture-f','capture-other') AND d.kind='dispatch'",
            [], |row| row.get(0),
        )?;
        assert_eq!(ids, 2);
        Ok(())
    }).unwrap();
}
