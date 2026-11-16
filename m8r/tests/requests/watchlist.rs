use m8r::models::{categories, watchlist_items};
use serial_test::serial;

use crate::support::{location, schedule, schedule_params, signed_in, Sandbox, ISSUE, PR};

const MR_URL: &str = "https://git.example.com/g/p/-/merge_requests/4521";
const ISSUE_URL: &str = "https://github.com/o/r/issues/1";

#[tokio::test]
#[serial]
async fn the_watchlist_is_added_to_changed_and_removed_from() {
    signed_in(|request, ctx| async move {
        let page = request.get("/watchlist").await.text();
        assert!(page.contains("Nothing is on the watchlist."), "{page}");
        // The form offers the categories that every m8r starts with.
        assert!(page.contains(r#"<option value="Issue">Issue</option>"#));
        assert!(page.contains(r#"<option value="Pull/Merge Request">Pull/Merge Request</option>"#));

        // Not an issue or a PR/MR: the form comes back with what was typed.
        let res = request
            .post("/watchlist")
            .form(&[
                ("url", "https://example.com/x"),
                ("category", PR),
                ("priority", "high"),
                ("review_requested", "true"),
                ("note", "why"),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("isn&#39;t the URL of an issue"), "{page}");
        assert!(page.contains(r#"value="https://example.com/x""#), "{page}");
        assert!(
            page.contains(r#"<option value="Pull/Merge Request" selected>"#),
            "{page}"
        );
        assert!(
            page.contains(r#"name="review_requested" value="true" checked"#),
            "{page}"
        );
        let listed = |db| async move { watchlist_items::Model::list(db, None).await.unwrap() };
        assert!(listed(&ctx.db).await.is_empty());

        let res = request
            .post("/watchlist")
            .form(&[
                ("url", MR_URL),
                ("category", PR),
                ("priority", "medium"),
                ("note", "Alice asked"),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/watchlist");
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(
                r#"<a href="https://git.example.com/g/p/-/merge_requests/4521">g/p!4521</a>"#
            ),
            "{page}"
        );
        assert!(page.contains("not checked yet"), "{page}");

        // A row's form changes the category, the priority and the note.
        let res = request
            .post("/watchlist/1")
            .form(&[
                ("category", ISSUE),
                ("priority", "high"),
                ("note", "Now urgent"),
            ])
            .await;
        assert_eq!(location(&res), "/watchlist");
        let (item, category) = watchlist_items::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(
            (
                item.priority.as_str(),
                item.note.as_str(),
                category.name.as_str()
            ),
            ("high", "Now urgent", ISSUE)
        );
        let res = request
            .post("/watchlist/1")
            .form(&[("category", ISSUE), ("priority", "low"), ("note", "")])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("The priority is medium or high."), "{page}");

        assert_eq!(request.post("/watchlist/1/remove").await.status_code(), 303);
        assert!(listed(&ctx.db).await.is_empty());
        assert_eq!(request.post("/watchlist/1/remove").await.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_review_request_is_a_checkbox_on_a_pr_or_mr() {
    signed_in(|request, ctx| async move {
        let requested = |id| {
            let db = ctx.db.clone();
            async move {
                let (item, _) = watchlist_items::Model::find(&db, id).await.unwrap();
                item.review_requested
            }
        };

        // Nobody reviews an issue.
        let res = request
            .post("/watchlist")
            .form(&[
                ("url", ISSUE_URL),
                ("category", ISSUE),
                ("priority", "medium"),
                ("review_requested", "true"),
                ("note", ""),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("Only a PR/MR"), "{page}");

        let res = request
            .post("/watchlist")
            .form(&[
                ("url", MR_URL),
                ("category", PR),
                ("priority", "medium"),
                ("review_requested", "true"),
                ("note", ""),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        let res = request
            .post("/watchlist")
            .form(&[
                ("url", ISSUE_URL),
                ("category", ISSUE),
                ("priority", "medium"),
                ("note", ""),
            ])
            .await;
        assert_eq!(res.status_code(), 303);

        // The MR's row has the checkbox, checked. The issue's row has none.
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(r#"name="review_requested" value="true" form="item-1" checked"#),
            "{page}"
        );
        assert!(!page.contains(r#"value="true" form="item-2""#), "{page}");

        // A row saved without the checkbox turns the request off, and with it, on again.
        let off = [("category", PR), ("priority", "medium"), ("note", "")];
        let on = [
            ("category", PR),
            ("priority", "medium"),
            ("note", ""),
            ("review_requested", "true"),
        ];
        assert_eq!(
            request.post("/watchlist/1").form(&off).await.status_code(),
            303
        );
        assert!(!requested(1).await);
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(r#"name="review_requested" value="true" form="item-1"> requested"#),
            "{page}"
        );
        assert_eq!(
            request.post("/watchlist/1").form(&on).await.status_code(),
            303
        );
        assert!(requested(1).await);

        let res = request.post("/watchlist/2").form(&on).await;
        assert!(res.text().contains("Only a PR/MR"));
        assert!(!requested(2).await);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn categories_are_added_renamed_and_removed_on_the_watchlists_page() {
    signed_in(|request, ctx| async move {
        let names = |db| async move {
            let categories = categories::Model::list(db).await.unwrap();
            categories.into_iter().map(|it| it.name).collect::<Vec<_>>()
        };
        assert_eq!(names(&ctx.db).await, [ISSUE, PR]);

        let res = request
            .post("/categories")
            .form(&[("name", " Epic ")])
            .await;
        assert_eq!(location(&res), "/watchlist");
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(r#"name="name" value="Epic" form="category-3""#),
            "{page}"
        );

        // A name is there once, has something in it, and fits in 255 characters.
        for (name, problem) in [
            ("Epic", "There&#39;s already a category"),
            ("  ", "Give the category a name."),
            (&"x".repeat(256), "can have 255 characters"),
        ] {
            let res = request.post("/categories").form(&[("name", name)]).await;
            assert_eq!(res.status_code(), 200);
            let page = res.text();
            assert!(page.contains(problem), "{name}: {page}");
        }
        assert!(request
            .post("/categories")
            .form(&[("name", &"x".repeat(255))])
            .await
            .status_code()
            .is_redirection());

        let res = request
            .post("/categories/3")
            .form(&[("name", "Release")])
            .await;
        assert_eq!(location(&res), "/watchlist");
        // Its own name is no other category's.
        let res = request
            .post("/categories/3")
            .form(&[("name", "Release")])
            .await;
        assert_eq!(res.status_code(), 303);
        let res = request.post("/categories/3").form(&[("name", ISSUE)]).await;
        assert!(res.text().contains("There&#39;s already a category"));
        assert_eq!(names(&ctx.db).await[2], "Release");

        // A category that an item or a schedule uses stays.
        request
            .post("/watchlist")
            .form(&[
                ("url", ISSUE_URL),
                ("category", "Release"),
                ("priority", "medium"),
            ])
            .await;
        let res = request.post("/categories/3/remove").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("is in use: by 1 watchlist items and 0 schedules"),
            "{page}"
        );
        request.post("/watchlist/1/remove").await;

        let sandbox = Sandbox::new("categories-in-use");
        let params = m8r::models::schedules::Params {
            category: "Release".to_string(),
            ..schedule_params(&sandbox.repo(), "")
        };
        let used = schedule(&ctx, &params).await;
        let res = request.post("/categories/3/remove").await;
        assert!(res.text().contains("by 0 watchlist items and 1 schedules"));
        request
            .post(&format!("/schedules/{}/remove", used.id))
            .await;

        assert_eq!(
            location(&request.post("/categories/3/remove").await),
            "/watchlist"
        );
        assert_eq!(names(&ctx.db).await.len(), 3);
        assert_eq!(
            request.post("/categories/3/remove").await.status_code(),
            404
        );
    })
    .await;
}
