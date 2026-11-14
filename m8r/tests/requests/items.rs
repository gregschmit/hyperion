use m8r::models::items;
use serial_test::serial;

use crate::support::{location, signed_in};

#[tokio::test]
#[serial]
async fn items_are_added_changed_and_removed() {
    signed_in(|request, ctx| async move {
        assert!(request
            .get("/items")
            .await
            .text()
            .contains("Nothing has priority."));

        // Not an issue or a PR/MR: the form comes back with what was typed.
        let res = request
            .post("/items")
            .form(&[
                ("url", "https://example.com/x"),
                ("priority", "high"),
                ("note", "why"),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("isn&#39;t the URL of an issue"), "{page}");
        assert!(page.contains(r#"value="https://example.com/x""#), "{page}");
        assert!(items::Model::list(&ctx.db).await.unwrap().is_empty());

        let res = request
            .post("/items")
            .form(&[
                ("url", "https://git.example.com/g/p/-/merge_requests/4521"),
                ("priority", "medium"),
                ("note", "Alice asked"),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        let page = request.get("/items").await.text();
        assert!(
            page.contains(
                r#"<a href="https://git.example.com/g/p/-/merge_requests/4521">g/p!4521</a>"#
            ),
            "{page}"
        );
        assert!(page.contains("not checked yet"), "{page}");

        let res = request
            .post("/items/1")
            .form(&[("priority", "high"), ("note", "Now urgent")])
            .await;
        assert_eq!(location(&res), "/items");
        let item = items::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(
            (item.priority.as_str(), item.note.as_str()),
            ("high", "Now urgent")
        );
        let res = request
            .post("/items/1")
            .form(&[("priority", "low"), ("note", "")])
            .await;
        assert_eq!(res.status_code(), 400);

        assert_eq!(request.post("/items/1/remove").await.status_code(), 303);
        assert!(items::Model::list(&ctx.db).await.unwrap().is_empty());
        assert_eq!(request.post("/items/1/remove").await.status_code(), 404);
    })
    .await;
}
