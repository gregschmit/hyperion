//! What a run's command gets on stdin.

use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        repos,
        schedules::Items,
        watchlist_items::{self, Category},
    },
    prompts::{Context, STARTER, VARIABLES},
};
use serial_test::serial;

use crate::support::{self, configure};

const REQUESTS: Items = Items::Of(Category::Requests);
const ISSUES: Items = Items::Of(Category::Issues);

/// Two repos and a watchlist of five. In the first repo: an issue, an urgent MR with a note,
/// an MR that someone asked for, and an urgent issue. In the second: one MR.
async fn watchlist(ctx: &AppContext) -> (repos::Model, repos::Model) {
    let one = support::repo(&ctx.db, "~/git/g/p", "https://git.example.com/g/p").await;
    let two = support::repo(&ctx.db, "~/git/o/r", "https://github.com/o/r").await;
    for (url, priority, note, requested) in [
        (
            "https://git.example.com/g/p/-/issues/1",
            "medium",
            "",
            false,
        ),
        (
            "https://git.example.com/g/p/-/merge_requests/2",
            "high",
            "Ship by <Friday>",
            false,
        ),
        (
            "https://git.example.com/g/p/-/merge_requests/3",
            "medium",
            "",
            true,
        ),
        ("https://git.example.com/g/p/-/issues/4", "high", "", false),
        ("https://github.com/o/r/pull/5", "medium", "Theirs", false),
    ] {
        let params = watchlist_items::Params {
            url: url.to_string(),
            priority: priority.to_string(),
            note: note.to_string(),
            requested,
        };
        let listed = watchlist_items::Model::create(&ctx.db, &params).await;
        listed.unwrap().unwrap();
    }
    (one, two)
}

/// What a prompt gets in `repo`, for a schedule that takes `items`, and `most` of them.
async fn context(
    ctx: &AppContext,
    repo: Option<&repos::Model>,
    items: Items,
    most: u32,
) -> Context {
    let loaded = Context::load(&ctx.db, repo, items, most, "/x").await;
    loaded.unwrap()
}

#[tokio::test]
#[serial]
async fn a_prompt_gets_each_part_where_it_says() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let (repo, _) = watchlist(ctx).await;
    configure(ctx, "Write to the reviewers in plain words.\n").await;

    let context = Context::load(&ctx.db, Some(&repo), REQUESTS, 0, "/data/artifacts/9")
        .await
        .unwrap();
    let prompt = context
        .compile(
            "Before.\n{{ m8r_prompt }}\n\n{{ config_prompt }}\nFollow these:\n\
             {{ watchlist }}\nAfter.",
        )
        .unwrap();

    // m8r's own part: where the agent is, where it reports, and how it keeps the watchlist.
    assert!(
        prompt.starts_with("Before.\nYou are running in m8r, a maintainer automation tool."),
        "{prompt}"
    );
    assert!(
        prompt.contains("make a directory in\n/data/artifacts/9, named"),
        "{prompt}"
    );
    assert!(
        prompt.contains("\n## How to use m8r's watchlist\n"),
        "{prompt}"
    );
    // Then yours. Then the repo's items of the category: the one that someone asked for
    // first, then the urgent one. Nothing is escaped: a prompt isn't HTML.
    assert!(
        prompt.ends_with(
            "they are the developer's.\n\n\
             Write to the reviewers in plain words.\n\
             Follow these:\n\
             - item 3 (medium, requested): https://git.example.com/g/p/-/merge_requests/3\n\
             - item 2 (high): https://git.example.com/g/p/-/merge_requests/2 Ship by <Friday>\n\
             After."
        ),
        "{prompt}"
    );
}

#[tokio::test]
#[serial]
async fn the_configurations_prompt_goes_in_as_it_was_written() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let (repo, _) = watchlist(ctx).await;
    // It's plain text, and no template: what looks like a tag is text too, and so is what
    // Tera couldn't read.
    let written = "Use {{ watchlist }} and {{ m8r_prompt }}.\n\
                   {% for %} {{ nope }} ${{ github.sha }} <b>";
    configure(ctx, written).await;
    let context = context(ctx, Some(&repo), REQUESTS, 0).await;

    assert_eq!(
        context.compile("[{{ config_prompt }}]").unwrap(),
        format!("[{written}]")
    );
}

#[tokio::test]
#[serial]
async fn a_run_gets_the_items_of_its_own_repo_that_its_schedule_takes() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let (one, two) = watchlist(ctx).await;
    let ids = "{% for item in watchlist_items %}{{ item.id }} {% endfor %}";

    // Every item of its repo: what someone asked for, then what's urgent, then the rest. No
    // other repo's.
    let all = context(ctx, Some(&one), Items::All, 0).await;
    assert_eq!(all.compile(ids).unwrap(), "3 2 4 1 ");
    assert_eq!(
        all.compile("{{ category }} of {{ repo }} ({{ repo_url }})")
            .unwrap(),
        "all of ~/git/g/p (https://git.example.com/g/p)"
    );
    let other = context(ctx, Some(&two), Items::All, 0).await;
    assert_eq!(
        other.compile("{{ repo }}:\n{{ watchlist }}").unwrap(),
        "~/git/o/r:\n- item 5 (medium): https://github.com/o/r/pull/5 Theirs"
    );

    // Or a category's.
    let issues = context(ctx, Some(&one), ISSUES, 0).await;
    assert_eq!(
        issues.compile("{{ category }}:\n{{ watchlist }}").unwrap(),
        "issues:\n\
         - item 4 (high): https://git.example.com/g/p/-/issues/4\n\
         - item 1 (medium): https://git.example.com/g/p/-/issues/1"
    );
    let requests = context(ctx, Some(&two), REQUESTS, 0).await;
    assert_eq!(requests.compile(ids).unwrap(), "5 ");
    assert_eq!(
        context(ctx, Some(&two), ISSUES, 0)
            .await
            .compile(ids)
            .unwrap(),
        ""
    );

    // The items are there to write the lines another way.
    let own = "{% for item in watchlist_items %}{{ item.id }} {{ item.priority }} \
               {{ item.category }} {{ item.requested }} {{ item.url }} {{ item.note }};\
               {% endfor %}";
    assert_eq!(
        context(ctx, Some(&one), REQUESTS, 0)
            .await
            .compile(own)
            .unwrap(),
        "3 medium pull/merge requests true https://git.example.com/g/p/-/merge_requests/3 ;\
         2 high pull/merge requests false https://git.example.com/g/p/-/merge_requests/2 \
         Ship by <Friday>;"
    );

    // A schedule that takes no items gets none, whatever is on the watchlist. And with no
    // repo at hand, as on the page of a schedule that has none yet, there are none.
    let none = context(ctx, Some(&one), Items::None, 0).await;
    assert_eq!(
        none.compile("[{{ category }}] {{ watchlist }}").unwrap(),
        "[] (nothing)"
    );
    assert_eq!(none.compile(own).unwrap(), "");
    let nowhere = context(ctx, None, Items::All, 0).await;
    assert_eq!(
        nowhere
            .compile("[{{ repo }}{{ repo_url }}] {{ watchlist }}")
            .unwrap(),
        "[] (nothing)"
    );
}

#[tokio::test]
#[serial]
async fn the_requested_items_are_first_and_have_a_list_of_their_own() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let (one, two) = watchlist(ctx).await;
    let both = "{{ requested_watchlist }}\n--\n{{ watchlist }}";
    let ids = "{% for item in requested_watchlist_items %}{{ item.id }}{% endfor %}/\
               {% for item in watchlist_items %}{{ item.id }}{% endfor %}";

    let all = context(ctx, Some(&one), Items::All, 0).await;
    assert_eq!(
        all.compile(both).unwrap(),
        "- item 3 (medium, requested): https://git.example.com/g/p/-/merge_requests/3\n\
         --\n\
         - item 3 (medium, requested): https://git.example.com/g/p/-/merge_requests/3\n\
         - item 2 (high): https://git.example.com/g/p/-/merge_requests/2 Ship by <Friday>\n\
         - item 4 (high): https://git.example.com/g/p/-/issues/4\n\
         - item 1 (medium): https://git.example.com/g/p/-/issues/1"
    );
    assert_eq!(all.compile(ids).unwrap(), "3/3241");

    // A schedule's cutoff keeps the first items, so what someone asked for is in.
    let two_at_most = context(ctx, Some(&one), Items::All, 2).await;
    assert_eq!(two_at_most.compile(ids).unwrap(), "3/32");
    let one_issue = context(ctx, Some(&one), ISSUES, 1).await;
    assert_eq!(one_issue.compile(ids).unwrap(), "/4");
    // Nobody asked for anything in the other repo, and a prompt can tell.
    let other = context(ctx, Some(&two), Items::All, 0).await;
    assert_eq!(
        other.compile("{{ requested_watchlist }}").unwrap(),
        "(nothing)"
    );
    assert_eq!(
        other
            .compile("{% if requested_watchlist_items %}first{% else %}none{% endif %}")
            .unwrap(),
        "none"
    );
}

#[tokio::test]
#[serial]
async fn m8rs_part_says_how_to_change_the_watchlist_and_holds_no_secret() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let (repo, _) = watchlist(ctx).await;
    let context = context(ctx, Some(&repo), Items::All, 0).await;

    let part = context.compile("{{ m8r_prompt }}").unwrap();

    assert_eq!(part, context.m8r_prompt().trim_end());
    // The address and the token are in the run's environment, by these names. The list is
    // the one of the run's own repo.
    let list = format!(
        r#"curl -sS "$M8R_URL/api/watchlist_items?repo_id={}" -H "Authorization: Bearer $M8R_TOKEN""#,
        repo.id
    );
    for request in [
        list.as_str(),
        r#"curl -sS "$M8R_URL/api/watchlist_items" -H "Authorization: Bearer $M8R_TOKEN" \"#,
        r#"curl -sS -X PATCH "$M8R_URL/api/watchlist_items/NUMBER" -H "Authorization: Bearer $M8R_TOKEN""#,
        r#"curl -sS -X DELETE "$M8R_URL/api/watchlist_items/NUMBER" -H "Authorization: Bearer $M8R_TOKEN""#,
    ] {
        assert!(part.contains(request), "{request}: {part}");
    }
    // An item's URL says the rest, so that's all that adding one takes.
    assert!(
        part.contains(
            r#"--json '{"url": "URL", "priority": "medium", "note": "Why the developer follows it"}'"#
        ),
        "{part}"
    );
    assert!(part.contains(r#"--json '{"requested": false}'"#), "{part}");
    assert!(part.ends_with("they are the developer's."), "{part}");
    // Nothing is left for a template to fill in.
    assert!(!part.contains("{artifacts_dir}") && !part.contains("{of_repo}"));

    // With no repo at hand, the list is the whole watchlist's.
    let nowhere = self::context(ctx, None, Items::All, 0).await;
    assert!(
        nowhere
            .m8r_prompt()
            .contains(r#"curl -sS "$M8R_URL/api/watchlist_items" -H "Authorization"#),
        "{}",
        nowhere.m8r_prompt()
    );
}

#[tokio::test]
#[serial]
async fn a_prompt_says_where_each_part_goes_or_goes_without_it() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    configure(ctx, "Write in plain words.\n").await;
    let context = context(ctx, None, Items::All, 0).await;

    assert_eq!(context.compile("Just triage.").unwrap(), "Just triage.");
    assert_eq!(context.compile("").unwrap(), "");
    // A part ends where its text does, so the prompt says what comes between two.
    assert_eq!(
        context.compile("{{ config_prompt }}\n\nTriage.").unwrap(),
        "Write in plain words.\n\nTriage."
    );

    // A new schedule's prompt holds m8r's part, yours and the watchlist, in this order.
    let starter = context.compile(STARTER).unwrap();
    assert!(starter.starts_with("You are running in m8r"), "{starter}");
    assert!(
        starter.ends_with(
            "they are the developer's.\n\nWrite in plain words.\n\n\
             The watchlist now:\n(nothing)\n\n"
        ),
        "{starter}"
    );

    // With no active configuration, that part is blank.
    let none = Context {
        config_prompt: String::new(),
        ..context
    };
    assert_eq!(none.compile("[{{ config_prompt }}]").unwrap(), "[]");
}

#[tokio::test]
#[serial]
async fn a_prompt_that_does_not_compile_says_why() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let context = context(ctx, None, Items::All, 0).await;

    let unknown = context.compile("{{ nope }}").unwrap_err();
    assert!(
        unknown.starts_with("The prompt doesn't compile."),
        "{unknown}"
    );
    assert!(unknown.contains("nope"), "{unknown}");
    // The names from before are no names now.
    assert!(context.compile("{{ system_message }}").is_err());
    assert!(context.compile("{{ skill }}").is_err());
}

#[tokio::test]
#[serial]
async fn every_variable_that_the_pages_name_is_one_that_compiles() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let (repo, _) = watchlist(ctx).await;
    configure(ctx, "Yours.").await;
    let context = context(ctx, Some(&repo), Items::All, 0).await;

    let names: Vec<_> = VARIABLES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names[..2], ["{{ m8r_prompt }}", "{{ config_prompt }}"]);
    for (variable, meaning) in VARIABLES {
        // The loop's page text has an ellipsis for its body.
        let template = variable.replace('…', "{{ item.url }}");
        let compiled = context.compile(&template);
        assert!(compiled.is_ok(), "{variable}: {compiled:?}");
        assert!(!compiled.unwrap().is_empty(), "{variable}");
        assert!(meaning.ends_with('.'), "{variable}");
    }
}
