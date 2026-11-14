use m8r::models::{runs, schedules};
use serial_test::serial;

use crate::support::{location, signed_in};

#[tokio::test]
#[serial]
async fn schedules_are_added_changed_run_and_removed() {
    signed_in(|request, ctx| async move {
        let here = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let page = request.get("/schedules").await.text();
        assert!(page.contains("No schedules."), "{page}");
        assert!(page.contains("No configuration is active"), "{page}");

        let res = request
            .post("/schedules")
            .form(&[
                ("operation", "triage"),
                ("repo", "/no/such/clone"),
                ("cron", "0 6 * * 1-5"),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        assert!(
            res.text().contains("isn&#39;t a directory"),
            "{}",
            res.text()
        );

        let res = request
            .post("/schedules")
            .form(&[
                ("operation", "triage"),
                ("repo", here.as_str()),
                ("cron", "0 6 * * 1-5"),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        let page = request.get("/schedules").await.text();
        assert!(page.contains(r#"value="0 6 * * 1-5""#), "{page}");

        let res = request
            .post("/schedules/1")
            .form(&[
                ("operation", "review"),
                ("repo", here.as_str()),
                ("cron", "30 13 * * *"),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        let schedule = schedules::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(
            (schedule.operation.as_str(), schedule.cron.as_str()),
            ("review", "30 13 * * *")
        );
        let res = request
            .post("/schedules/1")
            .form(&[
                ("operation", "review"),
                ("repo", here.as_str()),
                ("cron", "whenever"),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        assert!(res.text().contains("five fields"), "{}", res.text());

        // "Run now" queues the run, and the runs page shows it.
        let res = request.post("/schedules/1/run").await;
        assert_eq!(location(&res), "/runs");
        let queued = runs::Model::next(&ctx.db).await.unwrap().unwrap();
        assert_eq!(
            (queued.operation.as_str(), queued.repo.as_str()),
            ("review", here.as_str())
        );

        assert_eq!(request.post("/schedules/1/remove").await.status_code(), 303);
        assert!(schedules::Model::list(&ctx.db).await.unwrap().is_empty());
        assert_eq!(request.post("/schedules/1/run").await.status_code(), 404);
    })
    .await;
}
