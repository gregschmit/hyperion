//! The repos' page.

use m8r::models::repos;
use serial_test::serial;

use crate::support::{self, location, signed_in, watch};

#[tokio::test]
#[serial]
async fn repos_are_added_changed_and_removed() {
    signed_in(|request, ctx| async move {
        let here = std::env::current_dir().unwrap();
        let (src, tests) = (here.join("src"), here.join("tests"));
        let (src, tests) = (src.to_string_lossy(), tests.to_string_lossy());
        let page = request.get("/repos").await.text();
        assert!(page.contains("No repos yet"), "{page}");
        assert!(
            page.contains(r#"<a href="/repos" aria-current="page">Repos</a>"#),
            "{page}"
        );

        // A clone that isn't there: the form comes back with what was typed.
        let res = request
            .post("/repos")
            .form(&[
                ("path", "~/no/such/clone"),
                ("url", "https://git.example.com/g/p"),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("isn&#39;t a directory there"), "{page}");
        assert!(page.contains(r#"value="~/no/such/clone""#), "{page}");
        assert!(
            page.contains(r#"value="https://git.example.com/g/p""#),
            "{page}"
        );
        assert!(repos::Model::list(&ctx.db).await.unwrap().is_empty());

        // A clone and its project, as a remote writes it. And one with no project: this
        // directory is no clone, so nothing says one.
        let res = request
            .post("/repos")
            .form(&[("path", &*src), ("url", "git@git.example.com:g/p.git")])
            .await;
        assert_eq!(location(&res), "/repos");
        let res = request
            .post("/repos")
            .form(&[("path", &*tests), ("url", "")])
            .await;
        assert_eq!(location(&res), "/repos");
        let page = request.get("/repos").await.text();
        // A column goes by its name.
        assert!(
            page.contains("<th>Path</th><th>URL</th><th>Watchlist items</th><th>Schedules</th>"),
            "{page}"
        );
        let row = format!(
            r#"name="path" class="mono" value="{src}" form="repo-1" aria-label="Path" required>"#
        );
        assert!(page.contains(&row), "{page}");
        assert!(
            page.contains(r#"name="url" value="https://git.example.com/g/p" form="repo-1""#),
            "{page}"
        );
        assert!(
            page.contains(r#"name="url" value="" form="repo-2""#),
            "{page}"
        );
        // Each row says what's in the repo, and leads to its part of the watchlist.
        watch(&ctx, "https://git.example.com/g/p/-/issues/1").await;
        support::unchecked(&ctx.db, "triage", &src, "cat").await;
        let page = request.get("/repos").await.text();
        assert!(
            page.contains(r#"<a href="/watchlist?repo_id=1">1 item</a>"#),
            "{page}"
        );
        assert!(
            page.contains(r#"<a href="/watchlist?repo_id=2">0 items</a>"#),
            "{page}"
        );

        // A row's form changes the clone's path and the project. A project has one repo.
        let res = request
            .post("/repos/2")
            .form(&[("path", &*tests), ("url", "https://github.com/o/r/")])
            .await;
        assert_eq!(location(&res), "/repos");
        let changed = repos::Model::find(&ctx.db, 2).await.unwrap();
        assert_eq!(changed.url, "https://github.com/o/r");
        let res = request
            .post("/repos/2")
            .form(&[("path", &*tests), ("url", "https://git.example.com/g/p")])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("and a project has one repo."), "{page}");
        assert_eq!(
            repos::Model::find(&ctx.db, 2).await.unwrap().url,
            changed.url
        );

        // A repo that has an item or a schedule stays.
        let res = request.post("/repos/1/remove").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("is in use: by 1 watchlist items and 1 schedules."),
            "{page}"
        );
        assert_eq!(location(&request.post("/repos/2/remove").await), "/repos");
        assert_eq!(repos::Model::list(&ctx.db).await.unwrap().len(), 1);
        assert_eq!(request.post("/repos/2/remove").await.status_code(), 404);
        assert_eq!(
            request
                .post("/repos/2")
                .form(&[("path", &*tests), ("url", "")])
                .await
                .status_code(),
            404
        );
    })
    .await;
}
