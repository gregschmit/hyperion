use m8r::models::{runs, schedules};
use serial_test::serial;

use crate::support::{configure, location, signed_in, watch, ISSUE, PR};

#[tokio::test]
#[serial]
async fn schedules_are_added_changed_run_and_removed() {
    signed_in(|request, ctx| async move {
        let here = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let page = request.get("/schedules").await.text();
        assert!(page.contains("No schedules."), "{page}");
        assert!(page.contains("No configuration is active"), "{page}");

        // The form starts with the agent's command, and knows each operation's own.
        let page = request.get("/schedules/new").await.text();
        assert!(
            page.contains(r#"<option value="triage" data-command="cat" selected>triage</option>"#),
            "{page}"
        );
        assert!(
            page.contains(r#"<option value="other" data-command="">other</option>"#),
            "{page}"
        );
        assert!(
            page.contains(r#"name="command" class="wide mono" value="cat""#),
            "{page}"
        );
        // It offers no items or one category, and starts the prompt with m8r's own parts.
        assert!(
            page.contains(r#"<option value="">no items</option>"#),
            "{page}"
        );
        assert!(
            page.contains(r#"<option value="Pull/Merge Request">Pull/Merge Request</option>"#),
            "{page}"
        );
        assert!(
            page.contains(
                "{{ system_message }}\n\n{{ skill }}\n\nThe watchlist now:\n{{ watchlist }}"
            ),
            "{page}"
        );

        let triage = |repo: &'static str, hron: &'static str, command: &'static str| {
            [
                ("operation", "triage"),
                ("category", ""),
                ("repo", repo),
                ("hron", hron),
                ("command", command),
                ("prompt", "Triage the new issues."),
            ]
        };
        let res = request
            .post("/schedules")
            .form(&triage("/no/such/clone", "0 6 * * 1-5", "cat"))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("isn&#39;t a directory"), "{page}");
        // What was typed is still there.
        assert!(
            page.contains(">Triage the new issues.</textarea>"),
            "{page}"
        );

        let here: &'static str = Box::leak(here.into_boxed_str());
        let res = request
            .post("/schedules")
            .form(&triage(here, "0 6 * * 1-5", "cat"))
            .await;
        assert_eq!(location(&res), "/schedules");
        // The cron expression is in words now.
        let page = request.get("/schedules").await.text();
        assert!(page.contains("<td>every weekday at 06:00</td>"), "{page}");
        assert!(
            page.contains(r#"<span class="muted">no items</span>"#),
            "{page}"
        );

        let res = request
            .post("/schedules/1")
            .form(&[
                ("operation", "other"),
                ("category", PR),
                ("repo", here),
                ("hron", "every 12 hours"),
                ("command", r#"claude -p "Update the <changelog>.""#),
                ("prompt", "{{ watchlist }}"),
            ])
            .await;
        assert_eq!(location(&res), "/schedules/1");
        let (schedule, category) = schedules::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(
            (
                schedule.operation.as_str(),
                schedule.hron.as_str(),
                schedule.command.as_str(),
                schedule.prompt.as_str(),
            ),
            (
                "other",
                "every 12 hours from 00:00 to 23:59",
                r#"claude -p "Update the <changelog>.""#,
                "{{ watchlist }}",
            )
        );
        assert_eq!(category.unwrap().name, PR);
        let page = request.get("/schedules/1").await.text();
        assert!(
            page.contains(r#"value="claude -p &quot;Update the &lt;changelog&gt;.&quot;""#),
            "{page}"
        );
        assert!(
            page.contains(r#"<option value="Pull/Merge Request" selected>"#),
            "{page}"
        );

        // A time that m8r can't read: hron says where it went wrong.
        let review = |hron: &'static str, command: &'static str, category: &'static str| {
            [
                ("operation", "review"),
                ("category", category),
                ("repo", here),
                ("hron", hron),
                ("command", command),
                ("prompt", ""),
            ]
        };
        let res = request
            .post("/schedules/1")
            .form(&review("every day at 6am", "cat", PR))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("unknown keyword &#39;am&#39;"), "{page}");
        // A schedule has a command, and its category is one that's there.
        let res = request
            .post("/schedules/1")
            .form(&review("every day at 6:00", " ", PR))
            .await;
        assert!(res.text().contains("Say which command"), "{}", res.text());
        let res = request
            .post("/schedules/1")
            .form(&review("every day at 6:00", "cat", "Epic"))
            .await;
        let page = res.text();
        assert!(
            page.contains("There&#39;s no category &quot;Epic&quot;."),
            "{page}"
        );

        // "Run now" queues the schedule's run, and the runs page shows it.
        let res = request.post("/schedules/1/run").await;
        assert_eq!(location(&res), "/runs");
        let queued = runs::Model::next(&ctx.db).await.unwrap().unwrap();
        assert_eq!(queued.schedule_id, Some(1));
        assert_eq!(
            (
                queued.operation.as_str(),
                queued.repo.as_str(),
                queued.command.as_str()
            ),
            ("other", here, r#"claude -p "Update the <changelog>.""#)
        );
        let page = request.get("/runs").await.text();
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
        let here = std::env::current_dir().unwrap();
        configure(&ctx, "You are in m8r. Report in {{ artifacts_dir }}.").await;
        watch(&ctx, "https://github.com/o/r/issues/1", ISSUE).await;
        watch(&ctx, "https://github.com/o/r/pull/2", PR).await;

        // The variables are on the page, where the prompt is written.
        let page = request.get("/schedules/new").await.text();
        for variable in [
            "{{ system_message }}",
            "{{ skill }}",
            "{{ watchlist }}",
            "{{ category }}",
            "{{ artifacts_dir }}",
            "{% for item in watchlist_items %}",
        ] {
            assert!(
                page.contains(&format!("<code>{variable}")),
                "{variable}: {page}"
            );
        }
        // And the starting prompt shows compiled: the system message and the skill. A schedule
        // with no category gets no items.
        assert!(
            page.contains("You are in m8r. Report in /var/lib/m8r/artifacts/1."),
            "{page}"
        );
        assert!(page.contains("## How to use m8r&#39;s watchlist"), "{page}");
        assert!(page.contains("The watchlist now:\n(nothing)"), "{page}");
        assert!(!page.contains("- item 1 (Issue"), "{page}");

        // A schedule with a category gets that category's items.
        let res = request
            .post("/schedules")
            .form(&[
                ("operation", "review"),
                ("category", PR),
                ("repo", &*here.to_string_lossy()),
                ("hron", "every day at 9:00"),
                ("command", "cat"),
                (
                    "prompt",
                    "Review the {{ category }} items:\n{{ watchlist }}",
                ),
            ])
            .await;
        assert_eq!(location(&res), "/schedules");
        let page = request.get("/schedules/1").await.text();
        assert!(
            page.contains(
                "<pre>Review the Pull/Merge Request items:\n\
                 - item 2 (Pull/Merge Request, medium): https://github.com/o/r/pull/2</pre>"
            ),
            "{page}"
        );

        // A prompt that doesn't compile isn't saved, and Tera says why.
        let res = request
            .post("/schedules/1")
            .form(&[
                ("operation", "review"),
                ("category", PR),
                ("repo", &*here.to_string_lossy()),
                ("hron", "every day at 9:00"),
                ("command", "cat"),
                ("prompt", "{{ watchlist"),
            ])
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("The prompt doesn&#39;t compile."), "{page}");
        let (kept, _) = schedules::Model::find(&ctx.db, 1).await.unwrap();
        assert!(kept.prompt.starts_with("Review the"));

        // A blank prompt is fine: the command may need none.
        let res = request
            .post("/schedules/1")
            .form(&[
                ("operation", "other"),
                ("category", ""),
                ("repo", &*here.to_string_lossy()),
                ("hron", "every day at 9:00"),
                ("command", "make changelog"),
                ("prompt", ""),
            ])
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
        old.operation = Set("triage".to_string());
        old.repo = Set("/home/m8r/git/rxg".to_string());
        old.hron = Set("*/7 * * * *".to_string());
        old.command = Set("claude -p".to_string());
        old.insert(&ctx.db).await.unwrap();

        let page = request.get("/schedules").await.text();
        assert!(page.contains("<td>*/7 * * * *</td>"), "{page}");
        assert!(page.contains("m8r can't read this time"), "{page}");
    })
    .await;
}
