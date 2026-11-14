use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        configurations::Operation,
        runs::{self, Status},
    },
};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn the_queue_holds_one_waiting_run_for_an_operation_and_repo() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    let first = runs::Model::queue(db, Operation::Triage, "/a")
        .await
        .unwrap();
    assert_eq!(first.as_ref().unwrap().status, "queued");
    // The same again waits behind nothing: it's already waiting.
    assert!(runs::Model::queue(db, Operation::Triage, "/a")
        .await
        .unwrap()
        .is_none());
    assert!(runs::Model::queue(db, Operation::Review, "/a")
        .await
        .unwrap()
        .is_some());
    assert!(runs::Model::queue(db, Operation::Triage, "/b")
        .await
        .unwrap()
        .is_some());

    // The oldest goes first. Once it has started, the same can be queued again.
    let next = runs::Model::next(db).await.unwrap().unwrap();
    assert_eq!(next.id, first.unwrap().id);
    let running = next
        .into_active_model()
        .started(db, "the prompt".to_string())
        .await
        .unwrap();
    assert_eq!(running.status, "running");
    assert!(running.started_at.is_some());
    assert!(runs::Model::queue(db, Operation::Triage, "/a")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        runs::Model::next(db).await.unwrap().unwrap().operation,
        "review"
    );
}

#[tokio::test]
#[serial]
async fn a_run_that_was_cut_off_fails() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = runs::Model::queue(db, Operation::Develop, "/a")
        .await
        .unwrap()
        .unwrap();
    let run = run
        .into_active_model()
        .started(db, "p".to_string())
        .await
        .unwrap();
    let waiting = runs::Model::queue(db, Operation::Review, "/a")
        .await
        .unwrap()
        .unwrap();

    runs::Model::fail_running(db, "m8r stopped").await.unwrap();

    let run = runs::Model::find(db, run.id).await.unwrap();
    assert_eq!(run.status, Status::Failed.as_str());
    assert_eq!(run.error, "m8r stopped");
    assert!(run.finished_at.is_some());
    assert_eq!(
        runs::Model::find(db, waiting.id).await.unwrap().status,
        "queued"
    );
}
