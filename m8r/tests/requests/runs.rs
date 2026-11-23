use loco_rs::{
    prelude::*,
    testing::request::{request_with_config, RequestConfigBuilder},
};
use m8r::{
    app::App,
    models::runs::{self, Status},
    settings::Settings,
};
use serial_test::serial;

use crate::{
    stream::STREAM,
    support::{queue, share_path, signed_in, start, token, unchecked},
};

/// Where the review's comment is, which the run's report has a link to.
const COMMENT: &str = "https://git.example.com/g/p/-/merge_requests/1#note_5";

/// A run of the schedule `name` that's over. It printed markdown: a link to what it made, and
/// a script.
async fn reviewed(db: &DatabaseConnection, name: &str) -> runs::Model {
    let run = queue(db, name, "/home/m8r/git/rxg", "claude -p").await;
    let run = start(db, run, "The <prompt>.").await;
    let printed = format!(
        "# Review\n\nLooks **good**. See [my comment]({COMMENT}).\n\n<script>alert(1)</script>\n"
    );
    run.into_active_model()
        .finished(db, Status::Done, printed, String::new())
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
        // A column goes by its name, and one column has a run's three times.
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(
                "<th>ID</th><th>Name</th><th>Repo and command</th><th>Status</th>\
                 <th>Created, started and finished at</th></tr>"
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
        // It waits, so all that's known is when it was created.
        assert!(
            page.contains(r#"<td class="nowrap">created at 20"#),
            "{page}"
        );
        assert!(
            page.contains("<small>claude -p --model &lt;opus&gt;</small>"),
            "{page}"
        );

        // While a run is in progress, its page says how to read what it prints.
        let run = start(db, run, "The <prompt>.").await;
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(r#"<td class="nowrap">started at 20"#),
            "{page}"
        );
        let page = request.get("/runs/1").await.text();
        assert!(page.contains("<code>m8r task output id:1</code>"), "{page}");
        assert!(page.contains("Nothing yet."), "{page}");

        // It ends, and the list says how.
        run.into_active_model()
            .finished(
                db,
                Status::Failed,
                format!("# Done\n\nSee [my comment]({COMMENT}).\n\n<script>alert(1)</script>"),
                "the command exited with status 3".to_string(),
            )
            .await
            .unwrap();
        let page = request.get("/runs").await.text();
        assert!(
            page.contains(r#"<span class="status failed">failed</span>"#),
            "{page}"
        );
        assert!(page.contains(r#"<a href="/runs/1">1</a>"#), "{page}");
        // And when it ran, and for how long. It started at once, so the list leaves out when
        // it was created.
        let times = page.split_once(r#"<td class="nowrap">20"#).unwrap().1;
        let times = times.split_once("</td>").unwrap().0;
        assert!(
            times.contains(" – ") && times.ends_with(" (0 s)"),
            "{times}"
        );

        // A run's page shows its command, what it printed and the prompt it got.
        let res = request.get("/runs/1").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("<pre>claude -p --model &lt;opus&gt;</pre>"),
            "{page}"
        );
        assert!(page.contains("<h2>Prompt</h2>"), "{page}");
        assert!(page.contains("The &lt;prompt&gt;."), "{page}");
        assert!(page.contains("the command exited with status 3"), "{page}");
        assert!(!page.contains("m8r task output"), "{page}");
        assert!(page.contains(" · finished at 20"), "{page}");
        assert!(page.contains(" (0 s)</span></p>"), "{page}");

        // What it printed shows as markdown, and a tab away as it was printed. A link in it is
        // a link, and the HTML in it is text in both.
        assert!(page.contains("<h2>Output</h2>"), "{page}");
        for tab in [
            r#"<input type="radio" name="output" id="output-markdown" checked>"#,
            r#"<label for="output-markdown">Markdown</label>"#,
            r#"<label for="output-raw">Raw</label>"#,
        ] {
            assert!(page.contains(tab), "{tab}: {page}");
        }
        assert!(
            page.contains(&format!(
                "<div class=\"markdown\"><h1>Done</h1>\n\
                 <p>See <a href=\"{COMMENT}\">my comment</a>.</p>\n\
                 <pre><code>&lt;script&gt;alert(1)&lt;/script&gt;</code></pre>\n</div>"
            )),
            "{page}"
        );
        let raw = page.split_once("<div><pre># Done\n\n").unwrap().1;
        let raw = raw.split_once("</pre></div>").unwrap().0;
        assert!(raw.starts_with("See [my comment](https:"), "{raw}");
        assert!(raw.contains("\n\n&lt;script&gt;alert(1)&lt;"), "{raw}");
        assert!(!page.contains("<script>alert"), "{page}");

        assert_eq!(request.get("/runs/99").await.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_runs_page_has_a_share_link_to_hand_on() {
    signed_in(|request, ctx| async move {
        reviewed(&ctx.db, "review").await;

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
        assert!(page.contains("can read this page until 20"), "{page}");

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
async fn a_share_link_opens_its_runs_page_and_nothing_else() {
    // The server keeps cookies, so a session would show if the link started one.
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |request, ctx| async move {
        let db = &ctx.db;
        let run = reviewed(db, "review").await;
        let other = reviewed(db, "triage").await;
        // m8r has an error, which is none of a guest's business.
        m8r::models::errors::Model::raise(db, "gh", "m8r finds no such command.", None)
            .await
            .unwrap();
        assert_eq!(request.get("/runs/1").await.status_code(), 401);

        let link = share_path(&ctx, run.id, 60);
        let res = request.get(&link).await;
        assert_eq!(res.status_code(), 200);
        // The address holds the token: nothing keeps the page, and a link on it tells the
        // next site nothing. The browser keeps nothing either.
        assert_eq!(res.header("cache-control"), "no-store");
        assert_eq!(res.header("referrer-policy"), "no-referrer");
        assert!(res.maybe_header("set-cookie").is_none());
        let page = res.text();

        // The run, as its own page shows it. A link that it printed goes where it goes.
        assert!(page.contains("<h1>Run 1: review in"), "{page}");
        // Its command ended well, so it's done, and the page's styles have a color for that.
        assert!(
            page.contains(r#"<span class="status done">done</span>"#),
            "{page}"
        );
        assert!(page.contains(".status.done {"), "{page}");
        assert!(page.contains("<pre>claude -p</pre>"), "{page}");
        assert!(
            page.contains(&format!(
                "<h1>Review</h1>\n<p>Looks <strong>good</strong>. \
                 See <a href=\"{COMMENT}\">my comment</a>.</p>"
            )),
            "{page}"
        );
        assert!(page.contains("The &lt;prompt&gt;."), "{page}");
        assert!(!page.contains("<script"), "{page}");
        // And nothing of the rest of m8r: no way to another page, no stylesheet to ask for,
        // no errors, and no link to hand on.
        assert!(!page.contains("<nav>"), "{page}");
        assert!(!page.contains(r#"href="/"#), "{page}");
        assert!(
            page.contains("<style>") && !page.contains("m8r.css"),
            "{page}"
        );
        assert!(!page.contains("class=\"alert\""), "{page}");
        assert!(!page.contains("m8r finds no such command"), "{page}");
        assert!(!page.contains("<details"), "{page}");

        // The token fits that run's page. It opens no other run, no other page and no
        // stylesheet.
        let held = link.split_once("?token=").unwrap().1.to_string();
        for path in [
            "/runs/2",
            "/runs",
            "/",
            "/schedules",
            "/static/m8r.css",
            "/runs/1/",
        ] {
            let res = request.get(&format!("{path}?token={held}")).await;
            assert_eq!(res.status_code(), 401, "{path}");
        }
        // Not the API, and a guest changes nothing.
        let res = request.get("/api/runs/1").authorization_bearer(&held).await;
        assert_eq!(res.status_code(), 401);
        assert_eq!(request.post(&link).await.status_code(), 401);
        // A token that was changed is none, and a token for the API is no share link.
        let mut changed = link.clone();
        let last = if changed.ends_with('A') { 'B' } else { 'A' };
        changed.pop();
        changed.push(last);
        assert_eq!(request.get(&changed).await.status_code(), 401);
        let api = token(&ctx, "reader", "runs").await;
        let res = request.get(&format!("/runs/1?token={api}")).await;
        assert_eq!(res.status_code(), 401);
        // The link started no session: the same page without the token stays shut.
        assert_eq!(request.get("/runs/1").await.status_code(), 401);

        // The other run has a link of its own.
        let res = request.get(&share_path(&ctx, other.id, 60)).await;
        assert_eq!(res.status_code(), 200);
        assert!(res.text().contains("<h1>Run 2: triage in"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_share_link_stops_working_when_its_time_is_up() {
    request::<App, _, _>(|request, ctx| async move {
        let run = reviewed(&ctx.db, "review").await;
        let link = share_path(&ctx, run.id, 1);
        assert_eq!(request.get(&link).await.status_code(), 200);

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

        // A new link for the same run works.
        let link = share_path(&ctx, run.id, 60);
        assert_eq!(request.get(&link).await.status_code(), 200);
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

#[tokio::test]
#[serial]
async fn a_runs_page_shows_what_the_run_prints_as_it_comes() {
    signed_in(|request, ctx| async move {
        let db = &ctx.db;
        let run = queue(db, "review", "/home/m8r/git/rxg", "claude -p").await;
        let run = start(db, run, "The prompt.").await;
        // Where the run's command prints while the run is in progress.
        let printed = Settings::from_context(&ctx).unwrap().output_file(run.id);
        std::fs::create_dir_all(printed.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&printed);

        // The page has the part that changes, and a script that asks for that part again.
        let page = request.get("/runs/1").await.text();
        assert!(page.contains(r#"<div id="output" data-live>"#), "{page}");
        assert!(
            page.contains("address.searchParams.set('part', 'output');"),
            "{page}"
        );
        let part = || async { request.get("/runs/1?part=output").await.text() };
        let nothing = part().await;
        assert!(
            nothing
                .trim_start()
                .starts_with(r#"<div id="output" data-live>"#),
            "{nothing}"
        );
        assert!(nothing.contains("Nothing yet."), "{nothing}");
        // Only that part: no page around it.
        assert!(
            !nothing.contains("<html") && !nothing.contains("<h2>"),
            "{nothing}"
        );

        // Text shows as it is.
        std::fs::write(&printed, "step 1 <b>\nstep 2\n").unwrap();
        let text = part().await;
        assert!(
            text.contains("<pre class=\"live\">step 1 &lt;b&gt;\nstep 2\n</pre>"),
            "{text}"
        );

        // A stream shows as the agent's steps so far: what it said, and the tools that it
        // used, each in a line.
        let lines: Vec<&str> = STREAM.lines().collect();
        std::fs::write(&printed, lines[..5].join("\n")).unwrap();
        let steps = part().await;
        assert!(
            steps.contains(
                "<div class=\"markdown live\"><p>I'll read the MR first.</p>\n<ul>\n\
                 <li><strong>Bash</strong> <code>command: glab mr view 12 --comments, \
                 description: Read MR 12</code></li>\n\
                 <li><strong>Agent</strong> <code>description: Review MR 12, prompt: Review \
                 it.</code></li>\n</ul>\n</div>"
            ),
            "{steps}"
        );
        assert!(
            !steps.contains("toolu_1") && !steps.contains("Let me look."),
            "{steps}"
        );

        // A guest, who holds the run's share link, watches the same way, and gets no more.
        let link = share_path(&ctx, run.id, 60);
        let res = request.get(&format!("{link}&part=output")).await;
        assert_eq!(res.status_code(), 200);
        assert_eq!(res.header("cache-control"), "no-store");
        let shared = res.text();
        assert!(shared.contains("<strong>Agent</strong>"), "{shared}");
        let shared = request.get(&link).await.text();
        assert!(
            shared.contains("searchParams.set('part', 'output')"),
            "{shared}"
        );
        assert!(shared.contains("<strong>Agent</strong>"), "{shared}");
        assert!(!shared.contains("m8r task output"), "{shared}");
        std::fs::remove_file(&printed).unwrap();
        let waiting = request.get(&link).await.text();
        assert!(waiting.contains("Nothing yet."), "{waiting}");
        assert!(!waiting.contains("m8r task output"), "{waiting}");

        // The run ends. The part says so, so the script loads the page, which has the
        // report and no more script for it.
        run.into_active_model()
            .finished(db, Status::Done, "# Review".to_string(), String::new())
            .await
            .unwrap();
        let over = part().await;
        assert!(
            over.trim_start().starts_with(r#"<div id="output">"#),
            "{over}"
        );
        assert!(over.contains("<h1>Review</h1>"), "{over}");
        let page = request.get("/runs/1").await.text();
        assert!(!page.contains("searchParams"), "{page}");
        assert!(
            page.contains(r#"<div class="markdown"><h1>Review</h1>"#),
            "{page}"
        );
    })
    .await;
}
