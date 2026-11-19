use loco_rs::{boot::run_task, prelude::*, task};
use m8r::{
    app::App,
    models::runs::{self, Status},
};
use serial_test::serial;

use crate::support::{queue, start};

#[tokio::test]
#[serial]
async fn the_runs_that_are_not_over_are_listed_on_the_command_line() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let db = &ctx.db;
    let list =
        || async { run_task::<App>(ctx, Some(&"runs".to_string()), &task::Vars::default()).await };

    // With none, it says so.
    assert!(list().await.is_ok());
    assert!(runs::Model::unfinished(db).await.unwrap().is_empty());

    // One run is over, one is in progress, and one waits.
    let over = start(db, queue(db, "docs", "/a", "cat").await, "p").await;
    over.into_active_model()
        .finished(db, Status::Ok, String::new(), String::new())
        .await
        .unwrap();
    let in_progress = start(db, queue(db, "review", "/b", "cat").await, "p").await;
    let waiting = queue(db, "triage", "/c", "cat").await;

    assert!(list().await.is_ok());
    // The list has the last two, oldest first.
    let listed: Vec<_> = runs::Model::unfinished(db)
        .await
        .unwrap()
        .into_iter()
        .map(|run| (run.id, run.status))
        .collect();
    assert_eq!(
        listed,
        [
            (in_progress.id, "running".to_string()),
            (waiting.id, "queued".to_string())
        ]
    );
}
