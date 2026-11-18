use loco_rs::prelude::*;
use m8r::models::{
    artifacts, configurations,
    runs::{self, Status},
    schedules,
};
use serial_test::serial;

use crate::support::{queue, queue_all, queue_one, signed_in, start, unchecked};

/// Record that `run` reported an artifact: a link, or with a blank `url`, files.
async fn report(db: &DatabaseConnection, run: &runs::Model, name: &str, url: &str) {
    let mut artifact = artifacts::ActiveModel::new();
    artifact.run_id = Set(run.id);
    artifact.name = Set(name.to_string());
    artifact.url = Set(url.to_string());
    artifact.insert(db).await.unwrap();
}

async fn finish(db: &DatabaseConnection, run: runs::Model, status: Status) -> runs::Model {
    let run = run.into_active_model();
    run.finished(db, status, String::new(), String::new())
        .await
        .unwrap()
}

#[tokio::test]
#[serial]
async fn a_new_install_says_how_to_set_it_up() {
    signed_in(|request, ctx| async move {
        let res = request.get("/").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("has nothing to do yet"), "{page}");
        assert!(page.contains("sudo -iu m8r"), "{page}");
        assert!(
            page.contains(r#"<a href="/repos">add the repos</a>"#),
            "{page}"
        );
        assert!(
            page.contains("No schedules yet, so nothing to report."),
            "{page}"
        );

        let params = configurations::Params {
            name: "One".to_string(),
            ..configurations::Params::default()
        };
        configurations::Model::create(&ctx.db, &params)
            .await
            .unwrap()
            .unwrap();
        let page = request.get("/").await.text();
        assert!(!page.contains("has nothing to do yet"), "{page}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_report_shows_what_each_schedules_latest_run_reported() {
    signed_in(|request, ctx| async move {
        let db = &ctx.db;
        // Two schedules, each with a run that waits.
        let first = queue(db, "review", "~/git/rxg", "claude -p").await;
        queue(db, "docs", "~/git/infrastructure", "make docs").await;
        let (reviews, _) = schedules::Model::find(db, first.schedule_id.unwrap())
            .await
            .unwrap();

        let page = request.get("/").await.text();
        // Each has a part, by its name and its repo.
        for part in [
            r#"<a href="/schedules/1">review</a> in <span class="mono">~/git/rxg</span>"#,
            r#"<a href="/schedules/2">docs</a> in <span class="mono">~/git/infrastructure</span>"#,
        ] {
            assert!(page.contains(part), "{page}");
        }
        assert_eq!(page.matches("It hasn't run yet.").count(), 2, "{page}");

        // The review runs, and reports a link and a set of files.
        let first = start(db, first, "p").await;
        let link = "https://git.example.com/g/p/-/merge_requests/1#note_5";
        report(db, &first, "comment on rxg!1", link).await;
        report(db, &first, "review of <rxg!1>", "").await;
        let first = finish(db, first, Status::Ok).await;
        let page = request.get("/").await.text();
        assert!(
            page.contains(&format!(r#"<a href="{link}">comment on rxg!1</a>"#)),
            "{page}"
        );
        assert!(
            page.contains(r#"<a href="/artifacts/2">review of &lt;rxg!1&gt;</a>"#),
            "{page}"
        );
        assert!(page.contains(r#"<a href="/runs/1">run 1</a>"#), "{page}");
        assert_eq!(page.matches("It hasn't run yet.").count(), 1, "{page}");

        // They stay while the schedule's next run waits, and while it runs.
        let second = queue_one(db, &reviews).await;
        assert!(request.get("/").await.text().contains("comment on rxg!1"));
        let second = start(db, second, "p").await;
        assert!(request.get("/").await.text().contains("comment on rxg!1"));

        // That run is over: what it reported takes their place.
        report(db, &second, "comment on rxg!2", "https://git.example.com/x").await;
        finish(db, second, Status::Failed).await;
        let page = request.get("/").await.text();
        assert!(page.contains("comment on rxg!2"), "{page}");
        assert!(!page.contains("comment on rxg!1"), "{page}");
        assert!(
            page.contains(r#"<span class="status failed">failed</span>"#),
            "{page}"
        );

        // A run that couldn't start reported nothing, and takes nothing's place.
        let third = queue_one(db, &reviews).await;
        finish(db, third, Status::Failed).await;
        assert!(request.get("/").await.text().contains("comment on rxg!2"));

        // One that ran and reported nothing says so.
        let fourth = queue_one(db, &reviews).await;
        let fourth = start(db, fourth, "p").await;
        finish(db, fourth, Status::Ok).await;
        let page = request.get("/").await.text();
        assert!(page.contains("Its latest run reported nothing."), "{page}");
        assert!(!page.contains("comment on rxg!2"), "{page}");

        // The runs' own pages keep what each one reported.
        let page = request.get(&format!("/runs/{}", first.id)).await.text();
        assert!(page.contains("comment on rxg!1"), "{page}");

        // A schedule that's gone has nothing to report.
        schedules::Model::remove(db, reviews.id).await.unwrap();
        let page = request.get("/").await.text();
        assert!(!page.contains(r#"<a href="/schedules/1">"#), "{page}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_report_has_a_part_for_each_repo_of_a_schedule() {
    signed_in(|request, ctx| async move {
        let db = &ctx.db;
        let triage = unchecked(db, "triage", "~/git/rxg\n~/git/infrastructure", "claude -p").await;
        // The repos go by their clones' paths.
        let mut queued = queue_all(db, &triage).await;
        let (in_infrastructure, in_rxg) = (queued.remove(0), queued.remove(0));
        assert_eq!(in_rxg.repo, "~/git/rxg");

        // The run in one repo is over, and the other still waits.
        let in_rxg = start(db, in_rxg, "p").await;
        report(db, &in_rxg, "triage of rxg", "https://git.example.com/a").await;
        finish(db, in_rxg, Status::Ok).await;
        let page = request.get("/").await.text();
        let part = |repo: &str| {
            format!(r#"<a href="/schedules/1">triage</a> in <span class="mono">{repo}</span>"#)
        };
        assert!(page.contains(&part("~/git/rxg")), "{page}");
        assert!(page.contains(&part("~/git/infrastructure")), "{page}");
        assert!(page.contains("triage of rxg"), "{page}");
        assert_eq!(page.matches("It hasn't run yet.").count(), 1, "{page}");

        // Each repo shows what its own latest run reported.
        let in_infrastructure = start(db, in_infrastructure, "p").await;
        let link = "https://git.example.com/b";
        report(db, &in_infrastructure, "triage of infrastructure", link).await;
        finish(db, in_infrastructure, Status::Failed).await;
        let page = request.get("/").await.text();
        assert!(page.contains("triage of rxg"), "{page}");
        assert!(page.contains("triage of infrastructure"), "{page}");
        assert!(page.contains(r#"<a href="/runs/1">run 1</a>"#), "{page}");
        assert!(page.contains(r#"<a href="/runs/2">run 2</a>"#), "{page}");
    })
    .await;
}
