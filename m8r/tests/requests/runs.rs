use loco_rs::prelude::*;
use m8r::models::{
    configurations::Operation,
    runs::{self, Status},
};
use serial_test::serial;

use crate::support::signed_in;

#[tokio::test]
#[serial]
async fn the_runs_page_lists_the_queue_and_the_history() {
    signed_in(|request, ctx| async move {
        let page = request.get("/runs").await.text();
        assert!(page.contains("No runs yet."), "{page}");

        let run = runs::Model::queue(&ctx.db, Operation::Triage, "/home/m8r/git/rxg")
            .await
            .unwrap()
            .unwrap();
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(r#"<span class="status queued">queued</span>"#),
            "{page}"
        );
        assert!(page.contains("/home/m8r/git/rxg"), "{page}");

        // A run's page shows the prompt it got and what the agent printed, as text.
        let run = run
            .into_active_model()
            .started(&ctx.db, "The <prompt>.".to_string())
            .await
            .unwrap();
        run.into_active_model()
            .finished(
                &ctx.db,
                Status::Failed,
                "<script>alert(1)</script>".to_string(),
                "the agent exited with status 3".to_string(),
            )
            .await
            .unwrap();
        let res = request.get("/runs/1").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("The &lt;prompt&gt;."), "{page}");
        assert!(
            page.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;")
                || page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{page}"
        );
        assert!(!page.contains("<script>"), "{page}");
        assert!(page.contains("the agent exited with status 3"), "{page}");

        assert_eq!(request.get("/runs/99").await.status_code(), 404);
    })
    .await;
}
