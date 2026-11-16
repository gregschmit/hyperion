use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        runs::{self, Status},
        schedules,
    },
};
use serial_test::serial;

use crate::support::{queue, start};

#[tokio::test]
#[serial]
async fn the_queue_holds_one_waiting_run_for_a_schedule() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    let first = queue(db, "triage", "/a", "cat").await;
    assert_eq!(first.status, "queued");
    assert_eq!(
        (
            first.operation.as_str(),
            first.repo.as_str(),
            first.command.as_str()
        ),
        ("triage", "/a", "cat")
    );
    let (schedule, _) = schedules::Model::find(db, first.schedule_id.unwrap())
        .await
        .unwrap();
    // The same again waits behind nothing: it's already waiting, and that's the answer.
    let again = runs::Model::queue(db, &schedule).await.unwrap();
    assert_eq!(again.id, first.id);
    // Another schedule is another run, even one that looks the same.
    let other = queue(db, "triage", "/a", "cat").await;
    assert_ne!(other.id, first.id);

    // The oldest goes first. Once it has started, its schedule can queue again.
    let next = runs::Model::next(db).await.unwrap().unwrap();
    assert_eq!(next.id, first.id);
    let running = start(db, next, "the prompt").await;
    assert_eq!(running.status, "running");
    assert_eq!(running.prompt, "the prompt");
    assert!(running.started_at.is_some());
    assert!(runs::Model::is_running(db, running.id).await.unwrap());
    let later = runs::Model::queue(db, &schedule).await.unwrap();
    assert_ne!(later.id, first.id);
    assert_eq!(runs::Model::next(db).await.unwrap().unwrap().id, other.id);
}

#[tokio::test]
#[serial]
async fn a_run_takes_its_schedule_as_it_is_when_the_run_starts() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = queue(db, "triage", "/a", "cat").await;

    // The schedule changes while its run waits.
    let (schedule, _) = schedules::Model::find(db, run.schedule_id.unwrap())
        .await
        .unwrap();
    let mut schedule = schedule.into_active_model();
    schedule.command = Set("cat -n".to_string());
    schedule.update(db).await.unwrap();

    let run = start(db, run, "p").await;
    assert_eq!(run.command, "cat -n");

    // The history outlives the schedule.
    schedules::Model::remove(db, run.schedule_id.unwrap())
        .await
        .unwrap();
    let kept = runs::Model::find(db, run.id).await.unwrap();
    assert_eq!(kept.schedule_id, None);
    assert_eq!(
        (kept.command.as_str(), kept.prompt.as_str()),
        ("cat -n", "p")
    );
}

#[tokio::test]
#[serial]
async fn a_run_that_was_cut_off_fails() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = queue(db, "develop", "/a", "cat").await;
    let run = start(db, run, "p").await;
    let waiting = queue(db, "review", "/a", "cat").await;

    runs::Model::fail_running(db, "m8r stopped").await.unwrap();

    let run = runs::Model::find(db, run.id).await.unwrap();
    assert_eq!(run.status, Status::Failed.as_str());
    assert_eq!(run.error, "m8r stopped");
    assert!(run.finished_at.is_some());
    assert!(!runs::Model::is_running(db, run.id).await.unwrap());
    assert_eq!(
        runs::Model::find(db, waiting.id).await.unwrap().status,
        "queued"
    );
}
