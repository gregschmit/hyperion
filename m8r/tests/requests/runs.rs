use loco_rs::prelude::*;
use m8r::models::runs::Status;
use serial_test::serial;

use crate::support::{queue, signed_in, start};

#[tokio::test]
#[serial]
async fn the_runs_page_lists_the_queue_and_the_history() {
    signed_in(|request, ctx| async move {
        let page = request.get("/runs").await.text();
        assert!(page.contains("No runs yet."), "{page}");

        let command = "claude -p --model <opus>";
        let run = queue(&ctx.db, "triage", "/home/m8r/git/rxg", command).await;
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(r#"<span class="status queued">queued</span>"#),
            "{page}"
        );
        assert!(page.contains("/home/m8r/git/rxg"), "{page}");
        assert!(
            page.contains("<small>claude -p --model &lt;opus&gt;</small>"),
            "{page}"
        );

        // A run's page shows its command, the prompt it got and what it printed, as text.
        let run = start(&ctx.db, run, "The <prompt>.").await;
        run.into_active_model()
            .finished(
                &ctx.db,
                Status::Failed,
                "<script>alert(1)</script>".to_string(),
                "the command exited with status 3".to_string(),
            )
            .await
            .unwrap();
        let res = request.get("/runs/1").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("<pre>claude -p --model &lt;opus&gt;</pre>"),
            "{page}"
        );
        assert!(page.contains("The &lt;prompt&gt;."), "{page}");
        assert!(
            page.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;")
                || page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{page}"
        );
        assert!(!page.contains("<script>"), "{page}");
        assert!(page.contains("the command exited with status 3"), "{page}");

        assert_eq!(request.get("/runs/99").await.status_code(), 404);
    })
    .await;
}
