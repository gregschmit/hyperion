//! What a run's command gets on stdin.

use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::watchlist_items,
    prompts::{Context, STARTER, VARIABLES},
};
use serial_test::serial;

use crate::support::{category, configure, watch, ISSUE, PR};

/// A watchlist of three: an issue, an urgent MR with a note, and a PR to review.
async fn watchlist(ctx: &AppContext) {
    watch(ctx, "https://github.com/o/r/issues/1", ISSUE).await;
    for (url, priority, note, review_requested) in [
        (
            "https://git.example.com/g/p/-/merge_requests/2",
            "high",
            "Ship by <Friday>",
            false,
        ),
        ("https://github.com/o/r/pull/3", "medium", "", true),
    ] {
        let params = watchlist_items::Params {
            url: url.to_string(),
            category: PR.to_string(),
            priority: priority.to_string(),
            note: note.to_string(),
            review_requested,
        };
        let listed = watchlist_items::Model::create(&ctx.db, &params).await;
        listed.unwrap().unwrap();
    }
}

#[tokio::test]
#[serial]
async fn a_prompt_gets_each_part_where_it_says() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    watchlist(ctx).await;
    configure(ctx, "Write to {{ category }} people in plain words.\n").await;
    let prs = category(ctx, PR).await;

    let context = Context::load(&ctx.db, Some(&prs), "/data/artifacts/9")
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
    // Then yours, which is a template too. Then the category's items, the urgent one first,
    // and nothing escaped: a prompt isn't HTML.
    assert!(
        prompt.ends_with(
            "they are the developer's.\n\n\
             Write to Pull/Merge Request people in plain words.\n\
             Follow these:\n\
             - item 2 (Pull/Merge Request, high): \
             https://git.example.com/g/p/-/merge_requests/2 Ship by <Friday>\n\
             - item 3 (Pull/Merge Request, medium, review requested): \
             https://github.com/o/r/pull/3\n\
             After."
        ),
        "{prompt}"
    );
}

#[tokio::test]
#[serial]
async fn a_schedule_gets_the_items_of_its_category_and_none_without_one() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    watchlist(ctx).await;

    let issues = category(ctx, ISSUE).await;
    let context = Context::load(&ctx.db, Some(&issues), "/x").await.unwrap();
    assert_eq!(
        context.compile("{{ category }}:\n{{ watchlist }}").unwrap(),
        "Issue:\n- item 1 (Issue, medium): https://github.com/o/r/issues/1"
    );

    // The items are there to write the lines another way.
    let own = "{% for item in watchlist_items %}{{ item.id }} {{ item.priority }} \
               {{ item.category }} {{ item.review_requested }} {{ item.url }} {{ item.note }};\
               {% endfor %}";
    let prs = category(ctx, PR).await;
    let context = Context::load(&ctx.db, Some(&prs), "/x").await.unwrap();
    assert_eq!(
        context.compile(own).unwrap(),
        "2 high Pull/Merge Request false https://git.example.com/g/p/-/merge_requests/2 \
         Ship by <Friday>;3 medium Pull/Merge Request true https://github.com/o/r/pull/3 ;"
    );

    // A schedule with no category gets no items, whatever is on the watchlist.
    let context = Context::load(&ctx.db, None, "/x").await.unwrap();
    assert_eq!(
        context.compile("[{{ category }}] {{ watchlist }}").unwrap(),
        "[] (nothing)"
    );
    assert_eq!(context.compile(own).unwrap(), "");

    // A category with nothing in it says so too.
    let epics = m8r::models::categories::Params {
        name: "Epic".to_string(),
    };
    let epics = m8r::models::categories::Model::create(&ctx.db, &epics).await;
    let context = Context::load(&ctx.db, Some(&epics.unwrap().unwrap()), "/x")
        .await
        .unwrap();
    assert_eq!(context.compile("{{ watchlist }}").unwrap(), "(nothing)");
}

#[tokio::test]
#[serial]
async fn m8rs_part_says_how_to_change_the_watchlist_and_holds_no_secret() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let context = Context::load(&ctx.db, None, "/x").await.unwrap();

    let part = context.compile("{{ m8r_prompt }}").unwrap();

    assert_eq!(part, context.m8r_prompt().trim_end());
    // The address and the token are in the run's environment, by these names.
    for request in [
        r#"curl -sS "$M8R_URL/api/watchlist_items" -H "Authorization: Bearer $M8R_TOKEN""#,
        r#"curl -sS -X PATCH "$M8R_URL/api/watchlist_items/NUMBER" -H "Authorization: Bearer $M8R_TOKEN""#,
        r#"curl -sS -X DELETE "$M8R_URL/api/watchlist_items/NUMBER" -H "Authorization: Bearer $M8R_TOKEN""#,
    ] {
        assert!(part.contains(request), "{request}: {part}");
    }
    assert!(
        part.contains(r#"--json '{"review_requested": false}'"#),
        "{part}"
    );
    // It names the categories as they are now.
    assert!(
        part.contains("The categories are: Issue, Pull/Merge Request.\n"),
        "{part}"
    );
    assert!(part.ends_with("they are the developer's."), "{part}");
    // Nothing is left for a template to fill in.
    assert!(!part.contains("{artifacts_dir}") && !part.contains("{categories}"));
}

#[tokio::test]
#[serial]
async fn a_prompt_says_where_each_part_goes_or_goes_without_it() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    configure(ctx, "Report in {{ artifacts_dir }} too.\n").await;
    let context = Context::load(&ctx.db, None, "/x").await.unwrap();

    assert_eq!(context.compile("Just triage.").unwrap(), "Just triage.");
    assert_eq!(context.compile("").unwrap(), "");
    // A part ends where its text does, so the prompt says what comes between two.
    assert_eq!(
        context.compile("{{ config_prompt }}\n\nTriage.").unwrap(),
        "Report in /x too.\n\nTriage."
    );

    // A new schedule's prompt holds m8r's part, yours and the watchlist, in this order.
    let starter = context.compile(STARTER).unwrap();
    assert!(starter.starts_with("You are running in m8r"), "{starter}");
    assert!(
        starter.ends_with(
            "they are the developer's.\n\nReport in /x too.\n\nThe watchlist now:\n(nothing)\n\n"
        ),
        "{starter}"
    );

    // With no active configuration, that part is blank.
    let none = Context {
        config_prompt: String::new(),
        ..context.clone()
    };
    assert_eq!(none.compile("[{{ config_prompt }}]").unwrap(), "[]");
    // A configuration's prompt can use the rest, m8r's part too.
    let fuller = Context {
        config_prompt: "{{ watchlist }} in {{ artifacts_dir }}\n{{ m8r_prompt }}".to_string(),
        ..context
    };
    let compiled = fuller.compile("{{ config_prompt }}").unwrap();
    assert!(
        compiled.starts_with("(nothing) in /x\nYou are running in m8r"),
        "{compiled}"
    );
}

#[tokio::test]
#[serial]
async fn a_template_that_does_not_compile_says_which_one_and_why() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let context = Context::load(&ctx.db, None, "/x").await.unwrap();

    let unknown = context.compile("{{ nope }}").unwrap_err();
    assert!(
        unknown.starts_with("The prompt doesn't compile."),
        "{unknown}"
    );
    assert!(unknown.contains("nope"), "{unknown}");
    // The names from before are no names now.
    assert!(context.compile("{{ system_message }}").is_err());
    assert!(context.compile("{{ skill }}").is_err());

    let broken = Context {
        config_prompt: "{% for %}".to_string(),
        ..context
    };
    let problem = broken.compile("anything").unwrap_err();
    assert!(
        problem.starts_with("The configuration's prompt doesn't compile."),
        "{problem}"
    );
}

#[tokio::test]
#[serial]
async fn every_variable_that_the_pages_name_is_one_that_compiles() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    watchlist(ctx).await;
    let context = Context::load(&ctx.db, None, "/x").await.unwrap();

    let names: Vec<_> = VARIABLES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names[..2], ["{{ m8r_prompt }}", "{{ config_prompt }}"]);
    for (variable, meaning) in VARIABLES {
        // The loop's page text has an ellipsis for its body.
        let template = variable.replace('…', "{{ item.url }}");
        let compiled = context.compile(&template);
        assert!(compiled.is_ok(), "{variable}: {compiled:?}");
        assert!(meaning.ends_with('.'), "{variable}");
    }
}
