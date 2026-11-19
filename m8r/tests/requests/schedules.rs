use m8r::models::{runs, schedules};
use serial_test::serial;

use crate::support::{self, location, signed_in, watch, REQUESTS};

/// How many of a form's repos are checked.
fn checked_repos(page: &str) -> usize {
    let checked = |line: &&str| line.contains(r#"name="repos""#) && line.contains(" checked>");
    page.lines().filter(checked).count()
}

fn here() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[tokio::test]
#[serial]
async fn schedules_are_added_changed_run_and_removed() {
    signed_in(|request, ctx| async move {
        let here: &'static str = Box::leak(here().into_boxed_str());
        let page = request.get("/schedules").await.text();
        assert!(page.contains("No schedules."), "{page}");
        // What's missing is in the alert, on every page.
        assert!(
            page.contains("<strong>schedules:</strong> There&#39;s no schedule"),
            "{page}"
        );

        // The form starts with the agent's command and with every item, and its prompt with
        // m8r's own parts. m8r has no repo yet, and the form says where to add one.
        let page = request.get("/schedules/new").await.text();
        // A field goes by the name of its column.
        for label in [
            r#"<label for="name">Name "#,
            r#"<label for="category">Category "#,
            r#"<label for="max_items">Max items "#,
            r#"<label for="when">When "#,
            r#"<label for="enabled">Enabled "#,
            r#"<label for="command">Command "#,
            r#"<label for="prompt">Prompt "#,
        ] {
            assert!(page.contains(label), "{label}: {page}");
        }
        assert!(
            page.contains(r#"name="command" class="wide mono" value="cat""#),
            "{page}"
        );
        for option in [
            r#"<option value="">no items</option>"#,
            r#"<option value="all" selected>all</option>"#,
            r#"<option value="issues">issues</option>"#,
            r#"<option value="pull/merge requests">pull/merge requests</option>"#,
        ] {
            assert!(page.contains(option), "{option}: {page}");
        }
        assert!(
            page.contains(r#"name="max_items" value="0" min="0" step="1">"#),
            "{page}"
        );
        // A new schedule is enabled.
        assert!(
            page.contains(r#"name="enabled" value="true" checked>"#),
            "{page}"
        );
        assert!(
            page.contains(
                "<textarea id=\"prompt\" name=\"prompt\" rows=\"14\">{{ m8r_prompt }}\n\n\
                 The watchlist now:\n{{ watchlist }}"
            ),
            "{page}"
        );
        assert!(
            page.contains(r#"m8r has no repos yet. <a href="/repos">Add one</a> first."#),
            "{page}"
        );

        let triage = |repo: &'static str, when: &'static str, command: &'static str| {
            [
                ("name", "triage"),
                ("category", ""),
                ("max_items", ""),
                ("repos", repo),
                ("when", when),
                ("command", command),
                ("prompt", "Triage the new issues."),
                ("enabled", "true"),
            ]
        };
        // A clone that's there is no repo until it's one of m8r's.
        let res = request
            .post("/schedules")
            .form(&triage(here, "0 6 * * 1-5", "cat"))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("isn&#39;t one of m8r&#39;s repos. Add it to the repos first."),
            "{page}"
        );
        // What was typed is still there.
        assert!(
            page.contains(">Triage the new issues.</textarea>"),
            "{page}"
        );
        assert!(page.contains(r#"name="name" value="triage""#), "{page}");

        // With the repo, the form has a checkbox for it.
        support::repo(&ctx.db, here, "").await;
        let page = request.get("/schedules/new").await.text();
        let checkbox = format!(r#"<input type="checkbox" name="repos" value="{here}">"#);
        assert!(page.contains(&checkbox), "{page}");
        let res = request
            .post("/schedules")
            .form(&triage(here, "0 6 * * 1-5", "cat"))
            .await;
        assert_eq!(location(&res), "/schedules");
        // The cron expression is in words now.
        let page = request.get("/schedules").await.text();
        assert!(
            page.contains(r#"<a href="/schedules/1">triage</a>"#),
            "{page}"
        );
        assert!(page.contains("<td>every weekday at 06:00</td>"), "{page}");
        assert!(
            page.contains(r#"<span class="muted">no items</span>"#),
            "{page}"
        );

        let res = request
            .post("/schedules/1")
            .form(&[
                ("name", "docs"),
                ("category", REQUESTS),
                ("max_items", "50"),
                ("repos", here),
                ("when", "every 12 hours"),
                ("command", r#"claude -p "Update the <changelog>.""#),
                ("prompt", "{{ watchlist }}"),
                ("enabled", "true"),
            ])
            .await;
        assert_eq!(location(&res), "/schedules/1");
        let (schedule, repos) = schedules::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(
            (
                schedule.name.as_str(),
                schedule.category.as_str(),
                schedule.max_items(),
                schedule.when.as_str(),
                schedule.command.as_str(),
                schedule.prompt.as_str(),
            ),
            (
                "docs",
                REQUESTS,
                50,
                "every 12 hours from 00:00 to 23:59",
                r#"claude -p "Update the <changelog>.""#,
                "{{ watchlist }}",
            )
        );
        assert_eq!(repos[0].path, here);
        let page = request.get("/schedules/1").await.text();
        assert!(page.contains("<h1>docs</h1>"), "{page}");
        assert!(
            page.contains(r#"value="claude -p &quot;Update the &lt;changelog&gt;.&quot;""#),
            "{page}"
        );
        assert!(
            page.contains(r#"<option value="pull/merge requests" selected>"#),
            "{page}"
        );
        assert!(page.contains(r#"name="max_items" value="50""#), "{page}");
        let checked = format!(r#"<input type="checkbox" name="repos" value="{here}" checked>"#);
        assert!(page.contains(&checked), "{page}");
        let page = request.get("/schedules").await.text();
        assert!(
            page.contains(
                "<th>Category</th><th>Max items</th><th>Repos and command</th><th>When</th>"
            ),
            "{page}"
        );
        assert!(
            page.contains("<td class=\"nowrap\">pull/merge requests</td>\n    <td>50</td>"),
            "{page}"
        );

        // The list says whether a schedule is enabled, and turns it off and on. Off, it has no
        // next time, and its form says so too.
        assert!(
            page.contains("<th>Name</th><th>Enabled</th><th>Category</th>"),
            "{page}"
        );
        assert!(page.contains("<td>yes</td>"), "{page}");
        assert!(
            page.contains(r#"action="/schedules/1/disable"><button>Disable</button>"#),
            "{page}"
        );
        let res = request.post("/schedules/1/disable").await;
        assert_eq!(location(&res), "/schedules");
        let page = request.get("/schedules").await.text();
        assert!(
            page.contains(r#"<td><span class="status queued">no</span></td>"#),
            "{page}"
        );
        assert!(page.contains(r#"<td class="nowrap"></td>"#), "{page}");
        assert!(
            page.contains(
                r#"action="/schedules/1/enable"><button class="primary">Enable</button>"#
            ),
            "{page}"
        );
        let page = request.get("/schedules/1").await.text();
        assert!(page.contains(r#"name="enabled" value="true">"#), "{page}");
        let res = request.post("/schedules/1/enable").await;
        assert_eq!(location(&res), "/schedules");
        let page = request.get("/schedules/1").await.text();
        assert!(
            page.contains(r#"name="enabled" value="true" checked>"#),
            "{page}"
        );
        assert_eq!(
            request.post("/schedules/9/disable").await.status_code(),
            404
        );

        // A time that m8r can't read: hron says where it went wrong.
        let review = |when: &'static str, command: &'static str, category: &'static str| {
            [
                ("name", "review"),
                ("category", category),
                ("max_items", "0"),
                ("repos", here),
                ("when", when),
                ("command", command),
                ("prompt", ""),
                ("enabled", "true"),
            ]
        };
        let res = request
            .post("/schedules/1")
            .form(&review("every day at 6am", "cat", REQUESTS))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("unknown keyword &#39;am&#39;"), "{page}");
        // A schedule has a command, and it takes all the items, a category's, or none.
        let res = request
            .post("/schedules/1")
            .form(&review("every day at 6:00", " ", REQUESTS))
            .await;
        assert!(res.text().contains("Say which command"), "{}", res.text());
        let res = request
            .post("/schedules/1")
            .form(&review("every day at 6:00", "cat", "Epic"))
            .await;
        let page = res.text();
        assert!(
            page.contains("A schedule&#39;s category is all, issues or pull/merge requests"),
            "{page}"
        );
        // Its max items is a number, and it runs in a repo.
        let mut many = review("every day at 6:00", "cat", REQUESTS);
        many[2] = ("max_items", "many");
        let res = request.post("/schedules/1").form(&many).await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("Max items is a whole number: 0 for all of them."),
            "{page}"
        );
        let nowhere: Vec<_> = review("every day at 6:00", "cat", REQUESTS)
            .into_iter()
            .filter(|(field, _)| *field != "repos")
            .collect();
        let res = request.post("/schedules/1").form(&nowhere).await;
        let page = res.text();
        assert!(
            page.contains("Say which repos the schedule runs in."),
            "{page}"
        );
        // A name is one schedule's.
        let res = request
            .post("/schedules")
            .form(&review("every day at 6:00", "cat", REQUESTS))
            .await;
        assert_eq!(location(&res), "/schedules");
        let mut same = review("every day at 6:00", "cat", REQUESTS);
        same[0] = ("name", "docs");
        let res = request.post("/schedules/2").form(&same).await;
        let page = res.text();
        assert!(
            page.contains("There&#39;s already a schedule &quot;docs&quot;."),
            "{page}"
        );
        assert_eq!(request.post("/schedules/2/remove").await.status_code(), 303);

        // "Run now" queues the schedule's run, and the runs page shows it.
        let res = request.post("/schedules/1/run").await;
        assert_eq!(location(&res), "/runs");
        let queued = runs::Model::next(&ctx.db).await.unwrap().unwrap();
        assert_eq!(queued.schedule_id, Some(1));
        assert_eq!(
            (
                queued.name.as_str(),
                queued.repo.as_str(),
                queued.command.as_str()
            ),
            ("docs", here, r#"claude -p "Update the <changelog>.""#)
        );
        let page = request.get("/runs").await.text();
        assert!(page.contains("<td>docs</td>"), "{page}");
        assert!(
            page.contains("claude -p &quot;Update the &lt;changelog&gt;.&quot;"),
            "{page}"
        );

        // The run outlives its schedule.
        assert_eq!(request.post("/schedules/1/remove").await.status_code(), 303);
        assert!(schedules::Model::list(&ctx.db).await.unwrap().is_empty());
        assert_eq!(request.post("/schedules/1/run").await.status_code(), 404);
        let kept = runs::Model::find(&ctx.db, queued.id).await.unwrap();
        assert_eq!(kept.schedule_id, None);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_schedules_page_says_what_a_prompt_can_use_and_shows_it_compiled() {
    signed_in(|request, ctx| async move {
        let here = here();
        // The clone that the tests run in has an issue and a PR, and another repo has an MR.
        support::repo(&ctx.db, &here, "https://github.com/o/r").await;
        watch(&ctx, "https://github.com/o/r/issues/1").await;
        watch(&ctx, "https://github.com/o/r/pull/2").await;
        watch(&ctx, "https://git.example.com/g/p/-/merge_requests/3").await;

        // The variables are on the page, where the prompt is written.
        let page = request.get("/schedules/new").await.text();
        for variable in [
            "{{ m8r_prompt }}",
            "{{ watchlist }}",
            "{{ requested_watchlist }}",
            "{{ repo }}",
            "{{ repo_url }}",
            "{{ category }}",
            "{{ max_items }}",
            "{{ artifacts_dir }}",
            "{% for item in watchlist_items %}",
            "{% for item in requested_watchlist_items %}",
        ] {
            assert!(
                page.contains(&format!("<code>{variable}")),
                "{variable}: {page}"
            );
        }
        // And the starting prompt shows compiled: m8r's part, with a sample artifact
        // directory, and the watchlist. With no repo chosen yet, it gets no items.
        assert!(
            page.contains("<pre>You are running in m8r, a maintainer automation tool."),
            "{page}"
        );
        assert!(
            page.contains("make a directory in\n/var/lib/m8r/artifacts/1, named"),
            "{page}"
        );
        assert!(page.contains("## How to use m8r&#39;s watchlist"), "{page}");
        assert!(
            page.contains("With no repo chosen, it gets no items."),
            "{page}"
        );
        assert!(page.contains("The watchlist now:\n(nothing)"), "{page}");

        // A schedule gets the items of its repo, and of its category if it names one.
        let form = |name: &'static str, category: &'static str, prompt: &'static str| {
            [
                ("name", name.to_string()),
                ("category", category.to_string()),
                ("max_items", "0".to_string()),
                ("repos", here.clone()),
                ("when", "every day at 9:00".to_string()),
                ("command", "cat".to_string()),
                ("prompt", prompt.to_string()),
                ("enabled", "true".to_string()),
            ]
        };
        let res = request
            .post("/schedules")
            .form(&form(
                "review",
                REQUESTS,
                "Review the {{ category }} of {{ repo_url }}:\n{{ watchlist }}",
            ))
            .await;
        assert_eq!(location(&res), "/schedules");
        let page = request.get("/schedules/1").await.text();
        assert!(
            page.contains(
                "<pre>Review the pull/merge requests of https://github.com/o/r:\n\
                 - item 2 (medium): https://github.com/o/r/pull/2</pre>"
            ),
            "{page}"
        );
        // The page says whose items those are.
        let whose = format!(
            r#"as the next run would get it in
  <span class="mono">{here}</span>"#
        );
        assert!(page.contains(&whose), "{page}");

        // A prompt that doesn't compile isn't saved, and Tera says why.
        let res = request
            .post("/schedules/1")
            .form(&form("review", REQUESTS, "{{ watchlist"))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("The prompt doesn&#39;t compile."), "{page}");
        let (kept, _) = schedules::Model::find(&ctx.db, 1).await.unwrap();
        assert!(kept.prompt.starts_with("Review the"));

        // A blank prompt is fine: the command may need none.
        let res = request
            .post("/schedules/1")
            .form(&form("changelog", "", ""))
            .await;
        assert_eq!(location(&res), "/schedules/1");
        let page = request.get("/schedules/1").await.text();
        assert!(
            page.contains("Blank: the command gets nothing on stdin."),
            "{page}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_schedule_whose_time_cannot_be_read_says_so() {
    use loco_rs::prelude::*;

    signed_in(|request, ctx| async move {
        // From before m8r read times in words: a cron expression that hron has no words for.
        let mut old = schedules::ActiveModel::new();
        old.name = Set("triage".to_string());
        old.when = Set("*/7 * * * *".to_string());
        old.command = Set("claude -p".to_string());
        old.insert(&ctx.db).await.unwrap();

        let page = request.get("/schedules").await.text();
        assert!(page.contains("<td>*/7 * * * *</td>"), "{page}");
        assert!(page.contains("m8r can't read this time"), "{page}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_schedule_takes_several_repos_and_several_times() {
    signed_in(|request, ctx| async move {
        let here = here();
        let (src, tests) = (format!("{here}/src"), format!("{here}/tests"));
        for path in [&src, &tests, &format!("{here}/migration")] {
            support::repo(&ctx.db, path, "").await;
        }
        // The form has a checkbox for each repo, and a box for the times.
        let page = request.get("/schedules/new").await.text();
        assert_eq!(
            page.matches(r#"<input type="checkbox" name="repos""#)
                .count(),
            3
        );
        assert!(
            page.contains(r#"<textarea id="when" name="when" rows="3""#),
            "{page}"
        );

        // A checked box sends its repo. The times are one to a line, and a browser ends a
        // line with a return too.
        let form = |repos: &[&String], when: &'static str| {
            let mut form = vec![
                ("name", "triage".to_string()),
                ("category", String::new()),
                ("max_items", "0".to_string()),
                ("when", when.to_string()),
                ("command", "cat".to_string()),
                ("prompt", String::new()),
                ("enabled", "true".to_string()),
            ];
            form.extend(repos.iter().map(|repo| ("repos", (*repo).clone())));
            form
        };
        let res = request
            .post("/schedules")
            .form(&form(
                &[&tests, &src],
                "every weekday at 6:00\r\n0 12 * * 6",
            ))
            .await;
        assert_eq!(location(&res), "/schedules");

        // The list shows each repo and each time.
        let page = request.get("/schedules").await.text();
        assert!(
            page.contains(&format!(
                r#"<td class="mono">{src}<br>{tests}<br><small>cat</small></td>"#
            )),
            "{page}"
        );
        assert!(
            page.contains("<td>every weekday at 06:00<br>every saturday at 12:00</td>"),
            "{page}"
        );
        // And the form shows them again: its two repos are checked, and the third isn't.
        let page = request.get("/schedules/1").await.text();
        for repo in [&src, &tests] {
            let checked = format!(r#"name="repos" value="{repo}" checked>"#);
            assert!(page.contains(&checked), "{page}");
        }
        assert_eq!(checked_repos(&page), 2, "{page}");
        assert!(
            page.contains("required>every weekday at 06:00\nevery saturday at 12:00</textarea>"),
            "{page}"
        );

        // A line that m8r can't read: nothing is saved, and what was typed is still there.
        let res = request
            .post("/schedules/1")
            .form(&form(&[&src], "every day at 9:00\r\nwhenever"))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("  whenever\n"), "{page}");
        assert!(
            page.contains("required>every day at 9:00\r\nwhenever</textarea>"),
            "{page}"
        );
        assert_eq!(checked_repos(&page), 1, "{page}");
        let (_, kept) = schedules::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(kept.len(), 2);

        // "Run now" queues a run in each repo, and the runs page lists each.
        let res = request.post("/schedules/1/run").await;
        assert_eq!(location(&res), "/runs");
        let queued = runs::Model::next_batch(&ctx.db).await.unwrap();
        let repos: Vec<_> = queued.iter().map(|run| run.repo.as_str()).collect();
        assert_eq!(repos, [src.as_str(), tests.as_str()]);
        let page = request.get("/runs").await.text();
        for repo in [&src, &tests] {
            let run = format!(r#"<td class="mono">{repo}<br><small>cat</small></td>"#);
            assert!(page.contains(&run), "{page}");
        }
        assert_eq!(page.matches("<td>triage</td>").count(), 2, "{page}");
    })
    .await;
}
