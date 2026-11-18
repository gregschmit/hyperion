use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        repos,
        runs::{self, Status},
        schedules,
    },
};
use serial_test::serial;

use crate::support::{queue, queue_all, queue_one, start, unchecked};

#[tokio::test]
#[serial]
async fn the_queue_holds_one_waiting_run_for_a_schedule() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    let first = queue(db, "triage", "/a", "cat").await;
    assert_eq!(first.status, "queued");
    assert_eq!(
        (
            first.name.as_str(),
            first.repo.as_str(),
            first.command.as_str()
        ),
        ("triage", "/a", "cat")
    );
    let (schedule, _) = schedules::Model::find(db, first.schedule_id.unwrap())
        .await
        .unwrap();
    // The same again waits behind nothing: it's already waiting, and that's the answer.
    let again = queue_one(db, &schedule).await;
    assert_eq!(again.id, first.id);
    // It knows its repo, and keeps the path of the repo's clone.
    let repo = repos::Model::at(db, "/a").await.unwrap().unwrap();
    assert_eq!(first.repo_id, Some(repo.id));
    // Another schedule is another run, in the same repo too.
    let other = queue(db, "review", "/a", "cat").await;
    assert_ne!(other.id, first.id);
    assert_eq!(other.repo_id, Some(repo.id));

    // The oldest goes first. Once it has started, its schedule can queue again.
    let next = runs::Model::next(db).await.unwrap().unwrap();
    assert_eq!(next.id, first.id);
    let running = start(db, next, "the prompt").await;
    assert_eq!(running.status, "running");
    assert_eq!(running.prompt, "the prompt");
    assert!(running.started_at.is_some());
    assert!(runs::Model::is_running(db, running.id).await.unwrap());
    let later = queue_one(db, &schedule).await;
    assert_ne!(later.id, first.id);
    assert_eq!(runs::Model::next(db).await.unwrap().unwrap().id, other.id);
}

#[tokio::test]
#[serial]
async fn a_schedule_queues_a_run_for_each_of_its_repos_and_they_start_together() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let ids = |runs: &[runs::Model]| runs.iter().map(|run| run.id).collect::<Vec<_>>();

    let before = queue(db, "review", "/elsewhere", "cat").await;
    let schedule = unchecked(db, "triage", "/a\n/b\n/c", "cat").await;
    let queued = queue_all(db, &schedule).await;
    let repos: Vec<_> = queued.iter().map(|run| run.repo.as_str()).collect();
    assert_eq!(repos, ["/a", "/b", "/c"]);
    assert!(queued.iter().all(|run| {
        (run.schedule_id, run.status.as_str(), run.command.as_str())
            == (Some(schedule.id), "queued", "cat")
    }));
    let after = queue(db, "develop", "/a", "cat").await;
    // Queued again, they're the same runs: one waits in each repo.
    let again = queue_all(db, &schedule).await;
    assert_eq!(ids(&again), ids(&queued));

    // The schedules take turns, the one that has waited the longest first. A schedule's runs
    // go together.
    assert_eq!(
        ids(&runs::Model::next_batch(db).await.unwrap()),
        [before.id]
    );
    start(db, before, "p").await;
    let batch = runs::Model::next_batch(db).await.unwrap();
    assert_eq!(ids(&batch), ids(&queued));

    // One of them has started. The schedule queues again: a new run where none waits.
    let mut batch = batch;
    let started = start(db, batch.remove(1), "p").await;
    assert_eq!(started.repo, "/b");
    let next = queue_all(db, &schedule).await;
    assert_eq!((next[0].id, next[2].id), (queued[0].id, queued[2].id));
    assert!(next[1].id > after.id);
    // That one starts with the rest of its schedule's, though it was queued after another
    // schedule's run.
    assert_eq!(
        ids(&runs::Model::next_batch(db).await.unwrap()),
        [queued[0].id, queued[2].id, next[1].id]
    );

    // A run whose schedule is gone starts alone, to fail.
    schedules::Model::remove(db, schedule.id).await.unwrap();
    assert_eq!(
        ids(&runs::Model::next_batch(db).await.unwrap()),
        [queued[0].id]
    );
}

#[tokio::test]
#[serial]
async fn a_run_takes_its_schedule_and_its_repo_as_they_are_when_the_run_starts() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = queue(db, "triage", "/a", "cat").await;

    // The schedule changes while its run waits, and the repo's clone moves.
    let (schedule, repos) = schedules::Model::find(db, run.schedule_id.unwrap())
        .await
        .unwrap();
    let mut schedule = schedule.into_active_model();
    schedule.name = Set("mornings".to_string());
    schedule.command = Set("cat -n".to_string());
    schedule.update(db).await.unwrap();
    let mut repo = repos[0].clone().into_active_model();
    repo.path = Set("/moved".to_string());
    repo.update(db).await.unwrap();

    let run = start(db, run, "p").await;
    assert_eq!(
        (run.name.as_str(), run.command.as_str(), run.repo.as_str()),
        ("mornings", "cat -n", "/moved")
    );

    // The history outlives the schedule and the repo.
    schedules::Model::remove(db, run.schedule_id.unwrap())
        .await
        .unwrap();
    let repo = repos::Model::find(db, run.repo_id.unwrap()).await.unwrap();
    repo.removed(db).await.unwrap().unwrap();
    let kept = runs::Model::find(db, run.id).await.unwrap();
    assert_eq!((kept.schedule_id, kept.repo_id), (None, None));
    assert_eq!(
        (
            kept.name.as_str(),
            kept.command.as_str(),
            kept.repo.as_str(),
            kept.prompt.as_str()
        ),
        ("mornings", "cat -n", "/moved", "p")
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
