use loco_rs::prelude::*;
use m8r::models::watchlist_items::{self, Filter};
use serial_test::serial;

use crate::support::{self, location, signed_in, watch};

const MR_URL: &str = "https://git.example.com/g/p/-/merge_requests/4521";
const ISSUE_URL: &str = "https://github.com/o/r/issues/1";

#[tokio::test]
#[serial]
async fn the_watchlist_is_added_to_changed_and_removed_from() {
    signed_in(|request, ctx| async move {
        let page = request.get("/watchlist").await.text();
        assert!(page.contains("Nothing here."), "{page}");
        // An item's URL says what it is and where it is, so the form asks for neither.
        assert!(!page.contains(r#"name="category""#), "{page}");
        // A new item is medium, unless you say more.
        for option in [
            r#"<option value="medium" selected>medium</option>"#,
            r#"<option value="high">high</option>"#,
            r#"<option value="urgent">urgent</option>"#,
        ] {
            assert!(page.contains(option), "{option}\n{page}");
        }
        // A column goes by its name. An item's URL names its repo's project, so the repo
        // has no column of its own. The note gets the room that the others leave.
        assert!(
            page.contains(
                "<th>URL</th><th>Priority</th><th>Title</th><th>Requested</th>\
                 <th class=\"rest\">Note</th>"
            ),
            "{page}"
        );

        // Not an issue or a PR/MR: the form comes back with what was typed.
        let res = request
            .post("/watchlist")
            .form(&[
                ("url", "https://example.com/x"),
                ("priority", "high"),
                ("requested", "true"),
                ("note", "why"),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("isn&#39;t the URL of an issue"), "{page}");
        assert!(page.contains(r#"value="https://example.com/x""#), "{page}");
        assert!(page.contains(r#"<option value="high" selected>"#), "{page}");
        assert!(
            page.contains(r#"name="requested" value="true" checked"#),
            "{page}"
        );
        let listed = |db| async move {
            let listed = watchlist_items::Model::list(db, Filter::default()).await;
            listed.unwrap()
        };
        assert!(listed(&ctx.db).await.is_empty());

        // An item is in a repo, and m8r has none for this one yet.
        let mr = [
            ("url", MR_URL),
            ("priority", "medium"),
            ("note", "Alice asked"),
        ];
        let res = request.post("/watchlist").form(&mr).await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("m8r has no repo for https://git.example.com/g/p. Add that repo first"),
            "{page}"
        );
        assert!(listed(&ctx.db).await.is_empty());

        support::repo(&ctx.db, "~/git/g/p", "https://git.example.com/g/p").await;
        let res = request.post("/watchlist").form(&mr).await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/watchlist");
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(
                r#"<a href="https://git.example.com/g/p/-/merge_requests/4521">g/p!4521</a>"#
            ),
            "{page}"
        );
        // The page says what the URL says: what the item is.
        assert!(
            page.contains("</a><br><small>pull/merge requests</small>"),
            "{page}"
        );
        assert!(!page.contains("<td class=\"mono\">"), "{page}");
        assert!(page.contains("not checked yet"), "{page}");

        // The forge's title: its start, so that the note has room. All of it shows when you
        // point at it.
        let (item, _) = watchlist_items::Model::find(&ctx.db, 1).await.unwrap();
        let title = "Speed up the nightly review of <big> merge requests";
        item.into_active_model()
            .checked(&ctx.db, title.to_string(), chrono::Utc::now())
            .await
            .unwrap();
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(
                "<span title=\"Speed up the nightly review of &lt;big&gt; merge requests\">\
                 Speed up the nightly rev…</span>"
            ),
            "{page}"
        );

        // A row's form changes the priority and the note. The note has a box of two lines to
        // read it in, and it stays one line: a prompt has a line for each item.
        assert!(
            page.contains(
                r#"<textarea name="note" rows="2" form="item-1" aria-label="Note">Alice asked</textarea>"#
            ),
            "{page}"
        );
        let res = request
            .post("/watchlist/1")
            .form(&[("priority", "urgent"), ("note", " Now\r\n  urgent \n")])
            .await;
        assert_eq!(location(&res), "/watchlist");
        let (item, repo) = watchlist_items::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(
            (
                item.priority.as_str(),
                item.note.as_str(),
                repo.path.as_str()
            ),
            ("urgent", "Now urgent", "~/git/g/p")
        );
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(r#"<option value="urgent" selected>urgent</option>"#),
            "{page}"
        );
        let res = request
            .post("/watchlist/1")
            .form(&[("priority", "low"), ("note", "")])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("The priority is medium, high or urgent."),
            "{page}"
        );

        assert_eq!(request.post("/watchlist/1/remove").await.status_code(), 303);
        assert!(listed(&ctx.db).await.is_empty());
        assert_eq!(request.post("/watchlist/1/remove").await.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_request_is_a_checkbox_on_an_item() {
    signed_in(|request, ctx| async move {
        support::repo(&ctx.db, "~/git/g/p", "https://git.example.com/g/p").await;
        support::repo(&ctx.db, "~/git/o/r", "https://github.com/o/r").await;
        let requested = |id| {
            let db = ctx.db.clone();
            async move {
                let (item, _) = watchlist_items::Model::find(&db, id).await.unwrap();
                item.requested
            }
        };

        let res = request
            .post("/watchlist")
            .form(&[
                ("url", MR_URL),
                ("priority", "medium"),
                ("requested", "true"),
                ("note", ""),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        // Someone can ask you for an issue too. This one, nobody asked for.
        let res = request
            .post("/watchlist")
            .form(&[("url", ISSUE_URL), ("priority", "medium"), ("note", "")])
            .await;
        assert_eq!(res.status_code(), 303);

        // Each row has the checkbox: the MR's is checked, and the issue's isn't.
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(r#"name="requested" value="true" form="item-1" checked"#),
            "{page}"
        );
        assert!(
            page.contains(r#"name="requested" value="true" form="item-2"> requested"#),
            "{page}"
        );

        // A row saved without the checkbox turns the request off, and with it, on again.
        let off = [("priority", "medium"), ("note", "")];
        let on = [("priority", "medium"), ("note", ""), ("requested", "true")];
        assert_eq!(
            request.post("/watchlist/1").form(&off).await.status_code(),
            303
        );
        assert!(!requested(1).await);
        let page = request.get("/watchlist").await.text();
        assert!(
            page.contains(r#"name="requested" value="true" form="item-1"> requested"#),
            "{page}"
        );
        assert_eq!(
            request.post("/watchlist/1").form(&on).await.status_code(),
            303
        );
        assert!(requested(1).await);
        assert_eq!(
            request.post("/watchlist/2").form(&on).await.status_code(),
            303
        );
        assert!(requested(2).await);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_watchlist_shows_all_of_it_or_a_categorys_or_a_repos_part() {
    signed_in(|request, ctx| async move {
        let one = support::repo(&ctx.db, "~/git/g/p", "https://git.example.com/g/p").await;
        let two = support::repo(&ctx.db, "~/git/o/r", "https://github.com/o/r").await;
        watch(&ctx, MR_URL).await;
        watch(&ctx, "https://git.example.com/g/p/-/issues/7").await;
        watch(&ctx, ISSUE_URL).await;
        // Which of the three the page at `path` lists, and the page.
        let shown = |path: String| {
            let request = &request;
            async move {
                let page = request.get(&path).await.text();
                let has = |label: &str| page.contains(&format!(">{label}</a>"));
                ((has("g/p!4521"), has("g/p#7"), has("o/r#1")), page)
            }
        };

        // A tab for all of them, and one for each category. The one that's shown says so.
        let (all, page) = shown("/watchlist".to_string()).await;
        assert_eq!(all, (true, true, true));
        for tab in [
            r#"<a href="/watchlist" aria-current="page">all</a>"#,
            r#"<a href="/watchlist?category=issues">issues</a>"#,
            r#"<a href="/watchlist?category=pull%2Fmerge%20requests">pull/merge requests</a>"#,
        ] {
            assert!(page.contains(tab), "{tab}: {page}");
        }
        let (issues, page) = shown("/watchlist?category=issues".to_string()).await;
        assert_eq!(issues, (false, true, true));
        assert!(
            page.contains(r#"<a href="/watchlist?category=issues" aria-current="page">issues</a>"#),
            "{page}"
        );
        let requests = "/watchlist?category=pull%2Fmerge%20requests".to_string();
        assert_eq!(shown(requests).await.0, (true, false, false));

        // A repo's part, with the tabs for that repo. The two go together.
        let (of_one, page) = shown(format!("/watchlist?repo_id={}", one.id)).await;
        assert_eq!(of_one, (true, true, false));
        let tab = format!(
            r#"<a href="/watchlist?category=issues&amp;repo_id={}">"#,
            one.id
        );
        assert!(page.contains(&tab), "{page}");
        let chosen = format!(r#"<option value="{}" selected>~/git/g/p</option>"#, one.id);
        assert!(page.contains(&chosen), "{page}");
        let both = format!("/watchlist?category=issues&repo_id={}", two.id);
        let (of_both, page) = shown(both).await;
        assert_eq!(of_both, (false, false, true));
        // The form that chooses the repo keeps the category.
        assert!(
            page.contains(r#"<input type="hidden" name="category" value="issues">"#),
            "{page}"
        );

        // What the page can't read shows everything.
        let (anything, _) = shown("/watchlist?category=Epic&repo_id=x".to_string()).await;
        assert_eq!(anything, (true, true, true));
        // A repo's page leads here.
        let page = request.get("/repos").await.text();
        let link = format!(r#"<a href="/watchlist?repo_id={}">2 items</a>"#, one.id);
        assert!(page.contains(&link), "{page}");
    })
    .await;
}
