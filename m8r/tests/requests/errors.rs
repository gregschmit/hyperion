use loco_rs::prelude::*;
use m8r::models::{errors, runs::Status, schedules};
use serial_test::serial;

use crate::support::{queue, queue_one, signed_in, start};

#[tokio::test]
#[serial]
async fn every_page_shows_the_open_errors_and_one_page_lists_them_all() {
    signed_in(|request, ctx| async move {
        let db = &ctx.db;
        // A new m8r: no tick came yet, and nothing is scheduled.
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(r#"<a class="count" href="/errors">2 errors</a>"#),
            "{page}"
        );
        // The alert holds them all, and shows the first: its script shows the others in turn.
        assert!(
            page.contains("<li><strong>scheduler:</strong> The scheduler isn&#39;t running"),
            "{page}"
        );
        assert!(
            page.contains("<li hidden><strong>schedules:</strong> There&#39;s no schedule"),
            "{page}"
        );
        assert!(page.contains("setInterval"), "{page}");

        // Nobody clears an error: there's no button for it. A column goes by its name.
        let page = request.get("/errors").await.text();
        assert!(
            page.contains("<th>Created at</th><th>Subject</th><th>Message</th><th>Cleared at</th>"),
            "{page}"
        );
        assert!(
            page.contains(r#"<a href="/schedules">schedules</a>"#),
            "{page}"
        );
        let open = r#"<span class="status failed">not yet</span>"#;
        assert_eq!(page.matches(open).count(), 2, "{page}");
        assert!(!page.contains("<button"), "{page}");

        // It goes when what it's about is put right, at the next look. The list keeps it. One
        // error needs no script to take turns.
        let run = queue(db, "review", "~/git/rxg", "claude -p").await;
        let page = request.get("/errors").await.text();
        assert!(
            page.contains(r#"<a class="count" href="/errors">1 error</a>"#),
            "{page}"
        );
        assert!(!page.contains("<strong>schedules:</strong>"), "{page}");
        assert!(
            page.contains(r#"<a href="/schedules">schedules</a>"#),
            "{page}"
        );
        assert_eq!(page.matches(open).count(), 1, "{page}");
        assert!(!page.contains("setInterval"), "{page}");

        // A run fails: the error of its schedule in its repo leads to the run, where it can be
        // looked into.
        let run = start(db, run, "p").await.into_active_model();
        let why = "the command exited with status 1\nand more".to_string();
        let failed = run
            .finished(db, Status::Failed, String::new(), why)
            .await
            .unwrap();
        let page = request.get("/errors").await.text();
        assert!(
            page.contains(&format!(
                r#"<a href="/runs/{}">schedule 1 in ~/git/rxg</a>"#,
                failed.id
            )),
            "{page}"
        );
        assert!(
            page.contains(
                "<td class=\"message\">Run 1 (review) failed: the command exited with status \
                 1\nand more</td>"
            ),
            "{page}"
        );
        // The alert has the first line.
        assert!(
            page.contains(
                "<strong>schedule 1 in ~/git/rxg:</strong> Run 1 (review) failed: the command \
                 exited with status 1</li>"
            ),
            "{page}"
        );

        // The schedule's next run goes well, and its error is over.
        let (schedule, _) = schedules::Model::find(db, 1).await.unwrap();
        let next = queue_one(db, &schedule).await;
        let next = start(db, next, "p").await.into_active_model();
        next.finished(db, Status::Ok, String::new(), String::new())
            .await
            .unwrap();
        let page = request.get("/errors").await.text();
        assert!(!page.contains("<strong>schedule 1 in"), "{page}");
        assert!(
            page.contains(r#"<a href="/runs/1">schedule 1 in ~/git/rxg</a>"#),
            "{page}"
        );
        assert_eq!(page.matches(open).count(), 1, "{page}");

        // An error about a repo leads to the repos.
        errors::Model::raise(db, "repo ~/git/gone", "The clone isn't there.", None)
            .await
            .unwrap();
        let page = request.get("/errors").await.text();
        assert!(
            page.contains(r#"<a href="/repos">repo ~/git/gone</a>"#),
            "{page}"
        );
    })
    .await;
}
