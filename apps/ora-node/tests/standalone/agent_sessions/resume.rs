//! A resumed session fetches its immutable delivery ref over mTLS before launching the Agent.
use super::*;
use ora_test_support::git_server::git;
use pretty_assertions::assert_eq;

/// A fresh checkout resumes the prior final commit, retaining its own snapshot baseline and no
/// remote configuration; only the current session execution authorizes the fetch.
#[test]
fn resumed_session_restores_the_prior_revision_from_its_delivery_ref() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let clone = checkout(&fixture);
        let prior_path = fixture.path().join("prior-run");
        git(
            fixture.path(),
            &[
                "clone",
                "-q",
                fixture.path().join("main").to_str().unwrap(),
                prior_path.to_str().unwrap(),
            ],
        );
        fs::write(prior_path.join("prior.txt"), "work of the previous run\n").unwrap();
        git(
            &prior_path,
            &[
                "-c",
                "user.name=Prior",
                "-c",
                "user.email=prior@example.test",
                "add",
                "prior.txt",
            ],
        );
        git(
            &prior_path,
            &[
                "-c",
                "user.name=Prior",
                "-c",
                "user.email=prior@example.test",
                "commit",
                "-qm",
                "prior",
            ],
        );
        let final_commit = git(&prior_path, &["rev-parse", "HEAD"]);
        let delivery_ref = "refs/ora/deliveries/prior-work";
        git(
            &server.repository,
            &[
                "fetch",
                "-q",
                prior_path.to_str().unwrap(),
                &format!("HEAD:{delivery_ref}"),
            ],
        );
        let prior = PriorRevision {
            revision_id: RevisionId::new("revision-1"),
            final_commit: CommitId::new(&final_commit),
            base_commit: clone.commit.clone(),
            git_repository_url: server.url.clone(),
            delivery_ref: DeliveryRef::parse(delivery_ref).unwrap(),
        };
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ended = runtime.block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            let mut input = start("hello");
            input.payload.spec.prior_revision = Some(prior);
            send(
                &mut stream,
                ControllerToNodeMessage::StartAgentSession(input),
            )
            .await;
            let mut ending = false;
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::ThreadEvent(event) => {
                        send(&mut stream, ack(event.sequence.value())).await;
                        if !ending {
                            ending = true;
                            send(&mut stream, end()).await;
                        }
                    }
                    NodeToControllerMessage::AgentSessionEnded(ended) => {
                        send(&mut stream, ack(ended.sequence.value())).await;
                        break ended.payload;
                    }
                    NodeToControllerMessage::ExecutionStatus(_)
                    | NodeToControllerMessage::SessionCommandAccepted(_) => {}
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        child.terminate();
        let AgentSessionResult::AgentSessionEnded(ended) = ended;
        let checkout = PathBuf::from(clone.path.as_str());
        assert_eq!(
            (
                ended.reason,
                ended.detail,
                git(&checkout, &["rev-parse", "HEAD"]),
                git(&checkout, &["symbolic-ref", "HEAD"]),
                git(&checkout, &["remote"]),
                fs::read_to_string(checkout.join("prior.txt")).unwrap()
            ),
            (
                AgentSessionEndReason::UserEnded,
                None,
                final_commit,
                "refs/heads/main".into(),
                "".into(),
                "work of the previous run\n".into()
            )
        );
        assert!(
            server
                .server
                .served()
                .iter()
                .any(|r| r.execution_id.as_deref() == Some(EXECUTION))
        );
    });
}
