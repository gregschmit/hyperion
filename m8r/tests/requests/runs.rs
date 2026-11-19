use loco_rs::prelude::*;
use m8r::models::{
    artifacts,
    runs::{self, Status},
};
use serial_test::serial;

use crate::support::{queue, signed_in, start, unchecked};

/// Record that `run` reported an artifact: a link, or with a blank `url`, files.
async fn report(db: &DatabaseConnection, run: &runs::Model, name: &str, url: &str) {
    let mut artifact = artifacts::ActiveModel::new();
    artifact.run_id = Set(run.id);
    artifact.name = Set(name.to_string());
    artifact.url = Set(url.to_string());
    artifact.insert(db).await.unwrap();
}

#[tokio::test]
#[serial]
async fn a_new_install_says_how_to_set_it_up() {
    signed_in(|request, ctx| async move {
        // The runs are the home page.
        let res = request.get("/").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("<h1>Runs</h1>"), "{page}");
        assert!(page.contains("has nothing to do yet"), "{page}");
        assert!(page.contains("sudo -iu m8r"), "{page}");
        assert!(
            page.contains(r#"<a href="/repos">add the repos</a>"#),
            "{page}"
        );
        assert!(page.contains("No runs yet."), "{page}");

        // With a schedule, m8r is set up, and its first run is yours to queue.
        unchecked(&ctx.db, "triage", "~/git/rxg", "claude -p").await;
        let page = request.get("/").await.text();
        assert!(!page.contains("has nothing to do yet"), "{page}");
        assert!(page.contains("No runs yet."), "{page}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_runs_page_lists_the_queue_and_the_history() {
    signed_in(|request, ctx| async move {
        let db = &ctx.db;
        // A column goes by its name.
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(
                "<th>ID</th><th>Name</th><th>Repo and command</th><th>Status</th>\
                 <th>Created at</th><th>Started at</th><th>Finished at</th><th>Artifacts</th>"
            ),
            "{page}"
        );

        let command = "claude -p --model <opus>";
        let run = queue(db, "triage", "/home/m8r/git/rxg", command).await;
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(r#"<span class="status queued">queued</span>"#),
            "{page}"
        );
        assert!(page.contains("<td>triage</td>"), "{page}");
        assert!(page.contains("/home/m8r/git/rxg"), "{page}");
        assert!(
            page.contains("<small>claude -p --model &lt;opus&gt;</small>"),
            "{page}"
        );

        // While a run is in progress, its page says how to read what it prints.
        let run = start(db, run, "The <prompt>.").await;
        let page = request.get("/runs/1").await.text();
        assert!(page.contains("<code>m8r task output id:1</code>"), "{page}");

        // It reports a link and a set of files, and ends. The list has both: the link as it
        // is, and the files by their page.
        let link = "https://git.example.com/g/p/-/merge_requests/1#note_5";
        report(db, &run, "comment on rxg!1", link).await;
        report(db, &run, "review of <rxg!1>", "").await;
        run.into_active_model()
            .finished(
                db,
                Status::Failed,
                "<script>alert(1)</script>".to_string(),
                "the command exited with status 3".to_string(),
            )
            .await
            .unwrap();
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(r#"<span class="status failed">failed</span>"#),
            "{page}"
        );
        assert!(
            page.contains(&format!(r#"<a href="{link}">comment on rxg!1</a>"#)),
            "{page}"
        );
        assert!(
            page.contains(r#"<a href="/artifacts/2">review of &lt;rxg!1&gt;</a>"#),
            "{page}"
        );

        // A run's page shows its command, what it reported, what it printed and the prompt it
        // got, as text.
        let res = request.get("/runs/1").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("<pre>claude -p --model &lt;opus&gt;</pre>"),
            "{page}"
        );
        assert!(page.contains("comment on rxg!1"), "{page}");
        assert!(page.contains("<h2>Output</h2>"), "{page}");
        assert!(
            page.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;")
                || page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{page}"
        );
        assert!(!page.contains("<script>alert"), "{page}");
        assert!(page.contains("<h2>Prompt</h2>"), "{page}");
        assert!(page.contains("The &lt;prompt&gt;."), "{page}");
        assert!(page.contains("the command exited with status 3"), "{page}");
        assert!(!page.contains("m8r task output"), "{page}");

        assert_eq!(request.get("/runs/99").await.status_code(), 404);
    })
    .await;
}
