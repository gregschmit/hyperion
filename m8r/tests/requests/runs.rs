use std::fs;

use loco_rs::{
    prelude::*,
    testing::request::{request_with_config, RequestConfigBuilder},
};
use m8r::{
    app::App,
    models::{
        artifacts,
        runs::{self, Status},
    },
    settings::Settings,
};
use serial_test::serial;

use crate::support::{location, queue, share_path, signed_in, start, token, unchecked, write};

/// Record that `run` reported an artifact: a link, or with a blank `url`, files.
async fn report(db: &DatabaseConnection, run: &runs::Model, name: &str, url: &str) {
    let mut artifact = artifacts::ActiveModel::new();
    artifact.run_id = Set(run.id);
    artifact.name = Set(name.to_string());
    artifact.url = Set(url.to_string());
    artifact.insert(db).await.unwrap();
}

/// A run of the schedule `name` that's over. It reported three artifacts, which come in the
/// order of their names: `comment`, a link; `notes`, two files; and `review`, a page with a
/// picture. It printed markdown with a script in it.
async fn reviewed(ctx: &AppContext, name: &str) -> runs::Model {
    let db = &ctx.db;
    let run = queue(db, name, "/home/m8r/git/rxg", "claude -p").await;
    let run = start(db, run, "The <prompt>.").await;
    let dir = Settings::from_context(ctx).unwrap().artifacts_dir(run.id);
    let _ = fs::remove_dir_all(&dir);
    let link = "https://git.example.com/g/p/-/merge_requests/1#note_5";
    write(&dir.join("comment/link.txt"), link);
    write(&dir.join("notes/one.txt"), "1");
    write(&dir.join("notes/two.txt"), "2");
    let page = r#"<h1>Review</h1><img src="img/chart.svg">"#;
    write(&dir.join("review/index.html"), page);
    let picture = r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#;
    write(&dir.join("review/img/chart.svg"), picture);
    artifacts::Model::collect(db, run.id, &dir).await.unwrap();
    let printed = "# Review\n\nLooks **good**.\n\n<script>alert(1)</script>\n";
    run.into_active_model()
        .finished(db, Status::Ok, printed.to_string(), String::new())
        .await
        .unwrap()
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
                "# Done\n\n<script>alert(1)</script>".to_string(),
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
        // got.
        let res = request.get("/runs/1").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("<pre>claude -p --model &lt;opus&gt;</pre>"),
            "{page}"
        );
        assert!(page.contains("comment on rxg!1"), "{page}");
        assert!(page.contains("<h2>Prompt</h2>"), "{page}");
        assert!(page.contains("The &lt;prompt&gt;."), "{page}");
        assert!(page.contains("the command exited with status 3"), "{page}");
        assert!(!page.contains("m8r task output"), "{page}");

        // What it printed shows as markdown, and a tab away as it was printed. The HTML in it
        // is text in both.
        assert!(page.contains("<h2>Output</h2>"), "{page}");
        for tab in [
            r#"<input type="radio" name="output" id="output-markdown" checked>"#,
            r#"<label for="output-markdown">Markdown</label>"#,
            r#"<label for="output-raw">Raw</label>"#,
        ] {
            assert!(page.contains(tab), "{tab}: {page}");
        }
        assert!(
            page.contains(
                "<div class=\"markdown\"><h1>Done</h1>\n\
                 <pre><code>&lt;script&gt;alert(1)&lt;/script&gt;</code></pre>\n</div>"
            ),
            "{page}"
        );
        assert!(
            page.contains(
                "<div><pre># Done\n\n&lt;script&gt;alert(1)&lt;&#x2F;script&gt;</pre></div>"
            ) || page
                .contains("<div><pre># Done\n\n&lt;script&gt;alert(1)&lt;/script&gt;</pre></div>"),
            "{page}"
        );
        assert!(!page.contains("<script>alert"), "{page}");

        assert_eq!(request.get("/runs/99").await.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_runs_page_has_a_share_link_to_hand_on() {
    signed_in(|request, ctx| async move {
        reviewed(&ctx, "review").await;

        let page = request.get("/runs/1").await.text();
        // "Share" opens the link, and a script copies it.
        assert!(page.contains("<summary>Share</summary>"), "{page}");
        assert!(page.contains("navigator.clipboard"), "{page}");
        let start = r#"value="http://localhost:5150"#;
        let link = page.split_once(start).unwrap().1;
        let link = link.split_once('"').unwrap().0;
        // The link is this run's page, with a token that's short enough to read: 27
        // characters that an address takes as they are.
        let token = link.strip_prefix("/runs/1?token=").unwrap();
        assert_eq!(token.len(), 27, "{link}");
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{link}"
        );
        // It works for 72 hours, and the page says until when.
        assert!(
            page.contains("and the run's artifacts until\n    20"),
            "{page}"
        );

        // What the link shows is the same for everyone: the run, and no way to the rest.
        let shared = request.get(link).await.text();
        assert!(shared.contains("<h1>Run 1: review in"), "{shared}");
        assert!(!shared.contains("<nav>"), "{shared}");
        assert!(!shared.contains("<summary>Share</summary>"), "{shared}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_share_link_opens_its_run_and_the_runs_files_and_nothing_else() {
    // The server keeps cookies, as a guest's browser does.
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |mut request, ctx| async move {
        let db = &ctx.db;
        // Two runs. The first one's artifacts are 1 to 3, and the other's are 4 to 6.
        let run = reviewed(&ctx, "review").await;
        let other = reviewed(&ctx, "triage").await;
        // m8r has an error, which is none of a guest's business.
        m8r::models::errors::Model::raise(db, "gh", "m8r finds no such command.", None)
            .await
            .unwrap();
        for path in ["/runs/1", "/artifacts/3", "/artifacts/3/index.html"] {
            assert_eq!(request.get(path).await.status_code(), 401, "{path}");
        }

        let link = share_path(&ctx, run.id, 60);
        let res = request.get(&link).await;
        assert_eq!(res.status_code(), 200);
        // The address holds the token: nothing keeps the page, and a link on it tells the
        // next site nothing. The browser keeps what the link is good for.
        assert_eq!(res.header("cache-control"), "no-store");
        assert_eq!(res.header("referrer-policy"), "no-referrer");
        assert!(res.maybe_header("set-cookie").is_some());
        let page = res.text();

        // The run, as its own page shows it.
        assert!(page.contains("<h1>Run 1: review in"), "{page}");
        assert!(page.contains("<pre>claude -p</pre>"), "{page}");
        assert!(
            page.contains("<h1>Review</h1>\n<p>Looks <strong>good</strong>.</p>"),
            "{page}"
        );
        assert!(page.contains("The &lt;prompt&gt;."), "{page}");
        assert!(!page.contains("<script"), "{page}");
        // What it reported: a link goes where it goes, and the files are a page away.
        let comment =
            r#"<a href="https://git.example.com/g/p/-/merge_requests/1#note_5">comment</a>"#;
        assert!(page.contains(comment), "{page}");
        assert!(
            page.contains(r#"<a href="/artifacts/3">review</a>"#),
            "{page}"
        );
        // And nothing of the rest of m8r: no way to another page, no stylesheet to ask for,
        // no errors, and no link to hand on.
        assert!(!page.contains("<nav>"), "{page}");
        assert!(
            page.contains("<style>") && !page.contains("m8r.css"),
            "{page}"
        );
        assert!(!page.contains("class=\"alert\""), "{page}");
        assert!(!page.contains("m8r finds no such command"), "{page}");
        assert!(!page.contains("<details"), "{page}");

        // The run's files open: a page with what it needs, as inert as it is for anyone.
        let res = request.get("/artifacts/3").await;
        assert_eq!(location(&res), "/artifacts/3/index.html");
        let res = request.get("/artifacts/3/index.html").await;
        assert_eq!(res.status_code(), 200);
        let policy = res.header("content-security-policy");
        assert!(policy.to_str().unwrap().starts_with("default-src 'none';"));
        assert_eq!(res.header("cache-control"), "no-store");
        assert_eq!(res.header("referrer-policy"), "no-referrer");
        assert!(res.text().contains(r#"<img src="img/chart.svg">"#));
        let res = request.get("/artifacts/3/img/chart.svg").await;
        assert_eq!(res.status_code(), 200);
        // A list of files is a guest's page too: the way back to the run, and no other.
        let page = request.get("/artifacts/2").await.text();
        assert!(page.contains(r#"href="/artifacts/2/one.txt""#), "{page}");
        assert!(page.contains(r#"<a href="/runs/1">run 1</a>"#), "{page}");
        assert!(
            !page.contains("<nav>") && page.contains("<style>"),
            "{page}"
        );
        assert_eq!(request.get("/artifacts/2/two.txt").await.text(), "2");
        // The run's page opens by its plain address now, as a guest's.
        let page = request.get("/runs/1").await.text();
        assert!(page.contains("<h1>Run 1: review in"), "{page}");
        assert!(!page.contains("<nav>"), "{page}");

        // Nothing else opens, with the token or without it: not another run or its files,
        // no other page, and no stylesheet.
        let held = link.split_once("?token=").unwrap().1.to_string();
        for path in [
            "/runs/2",
            "/artifacts/6",
            "/artifacts/6/index.html",
            "/artifacts/99",
            "/runs",
            "/",
            "/schedules",
            "/static/m8r.css",
            "/runs/1/",
        ] {
            assert_eq!(request.get(path).await.status_code(), 401, "{path}");
            let res = request.get(&format!("{path}?token={held}")).await;
            assert_eq!(res.status_code(), 401, "{path} with the token");
        }
        // Not the API, and a guest changes nothing.
        let res = request.get("/api/runs/1").authorization_bearer(&held).await;
        assert_eq!(res.status_code(), 401);
        for path in [link.as_str(), "/runs/1", "/schedules/1/run"] {
            assert_eq!(request.post(path).await.status_code(), 401, "{path}");
        }
        // A token that was changed is none, and a token for the API is no share link.
        let mut changed = link.clone();
        let last = if changed.ends_with('A') { 'B' } else { 'A' };
        changed.pop();
        changed.push(last);
        assert_eq!(request.get(&changed).await.status_code(), 401);
        let api = token(&ctx, "reader", "runs").await;
        let res = request.get(&format!("/runs/1?token={api}")).await;
        assert_eq!(res.status_code(), 401);

        // The other run's link makes the browser a guest of that run too.
        let res = request.get(&share_path(&ctx, other.id, 60)).await;
        assert_eq!(res.status_code(), 200);
        for path in ["/artifacts/6/index.html", "/runs/2", "/runs/1"] {
            assert_eq!(request.get(path).await.status_code(), 200, "{path}");
        }

        // A browser that never opened a link gets no file: a file's address holds no token.
        // The link still opens the page.
        request.clear_cookies();
        for path in ["/runs/1", "/artifacts/3/index.html", "/artifacts/2"] {
            assert_eq!(request.get(path).await.status_code(), 401, "{path}");
        }
        assert_eq!(request.get(&link).await.status_code(), 200);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_share_link_stops_working_when_its_time_is_up() {
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |request, ctx| async move {
        let run = reviewed(&ctx, "review").await;
        let link = share_path(&ctx, run.id, 1);
        assert_eq!(request.get(&link).await.status_code(), 200);
        let file = "/artifacts/3/index.html";
        assert_eq!(request.get(file).await.status_code(), 200);

        // A link's time is in whole seconds, so wait it out.
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;

        // The link says that its time is up. Its holder can make no link on the server, so
        // the page doesn't say how.
        let res = request.get(&link).await;
        assert_eq!(res.status_code(), 401);
        let page = res.text();
        assert!(page.contains("That link to a run has expired."), "{page}");
        assert!(!page.contains("m8r task link"), "{page}");
        assert!(!page.contains("Review"), "{page}");
        // And the browser that opened it is no guest any more.
        for path in ["/runs/1", file] {
            let res = request.get(path).await;
            assert_eq!(res.status_code(), 401, "{path}");
            assert!(res.text().contains("open their link again"), "{path}");
        }

        // A new link for the same run works.
        let link = share_path(&ctx, run.id, 60);
        assert_eq!(request.get(&link).await.status_code(), 200);
        assert_eq!(request.get(file).await.status_code(), 200);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_guest_sees_a_run_that_is_not_over_as_far_as_it_is() {
    request::<App, _, _>(|request, ctx| async move {
        let run = queue(&ctx.db, "review", "/home/m8r/git/rxg", "claude -p").await;
        let run = start(&ctx.db, run, "p").await;

        let page = request.get(&share_path(&ctx, run.id, 60)).await.text();
        assert!(
            page.contains(r#"<span class="status running">running</span>"#),
            "{page}"
        );
        // A command on the server is no use to a guest.
        assert!(page.contains("Nothing yet."), "{page}");
        assert!(!page.contains("m8r task output"), "{page}");
    })
    .await;
}
