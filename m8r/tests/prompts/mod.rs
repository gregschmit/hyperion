//! What a run's command gets on stdin.

use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::watchlist_items,
    prompts::{Context, DEFAULT_SYSTEM_MESSAGE, STARTER, VARIABLES},
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
async fn a_prompt_gets_the_system_message_and_the_watchlist_where_it_says() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    watchlist(ctx).await;
    configure(ctx, DEFAULT_SYSTEM_MESSAGE).await;
    let prs = category(ctx, PR).await;

    let context = Context::load(&ctx.db, Some(&prs), "/data/artifacts/9")
        .await
        .unwrap();
    let prompt = context
        .compile("Before.\n{{ system_message }}\nFollow these:\n{{ watchlist }}\nAfter.")
        .unwrap();

    assert!(
        prompt.starts_with("Before.\nYou are running in m8r"),
        "{prompt}"
    );
    assert!(
        prompt.contains("make a directory in\n/data/artifacts/9, named"),
        "{prompt}"
    );
    // The category's items, the urgent one first, and nothing escaped: a prompt isn't HTML.
    assert!(
        prompt.ends_with(
            "only its link.\nFollow these:\n\
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
async fn the_skill_says_how_to_change_the_watchlist_and_holds_no_secret() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let context = Context::load(&ctx.db, None, "/x").await.unwrap();

    let skill = context.compile("{{ skill }}").unwrap();

    assert!(
        skill.starts_with("## How to use m8r's watchlist\n"),
        "{skill}"
    );
    // The address and the token are in the run's environment, by these names.
    for request in [
        r#"curl -sS "$M8R_URL/api/watchlist_items" -H "Authorization: Bearer $M8R_TOKEN""#,
        r#"curl -sS -X PATCH "$M8R_URL/api/watchlist_items/NUMBER" -H "Authorization: Bearer $M8R_TOKEN""#,
        r#"curl -sS -X DELETE "$M8R_URL/api/watchlist_items/NUMBER" -H "Authorization: Bearer $M8R_TOKEN""#,
    ] {
        assert!(skill.contains(request), "{request}: {skill}");
    }
    assert!(
        skill.contains(r#"--json '{"review_requested": false}'"#),
        "{skill}"
    );
    // It names the categories as they are now.
    assert!(
        skill.contains("The categories are: Issue, Pull/Merge Request.\n"),
        "{skill}"
    );
    assert!(skill.ends_with("they are the developer's."), "{skill}");
}

#[tokio::test]
#[serial]
async fn a_prompt_says_where_each_part_goes_or_goes_without_it() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    configure(ctx, "Report to {{ artifacts_dir }}.\n").await;
    let context = Context::load(&ctx.db, None, "/x").await.unwrap();

    assert_eq!(context.compile("Just triage.").unwrap(), "Just triage.");
    assert_eq!(context.compile("").unwrap(), "");
    // A part ends where its text does, so the prompt says what comes between two.
    assert_eq!(
        context.compile("{{ system_message }}\n\nTriage.").unwrap(),
        "Report to /x.\n\nTriage."
    );

    // A new schedule's prompt holds all three, in this order.
    let starter = context.compile(STARTER).unwrap();
    assert!(
        starter.starts_with("Report to /x.\n\n## How to use m8r's watchlist\n"),
        "{starter}"
    );
    assert!(
        starter.ends_with("they are the developer's.\n\nThe watchlist now:\n(nothing)\n\n"),
        "{starter}"
    );

    // With no active configuration there's no system message.
    let none = Context {
        system_message: String::new(),
        ..context.clone()
    };
    assert_eq!(none.compile("[{{ system_message }}]").unwrap(), "[]");
    // The system message can use the rest.
    let fuller = Context {
        system_message: "{{ watchlist }} in {{ artifacts_dir }}".to_string(),
        ..context
    };
    assert_eq!(
        fuller.compile("{{ system_message }}").unwrap(),
        "(nothing) in /x"
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

    let broken = Context {
        system_message: "{% for %}".to_string(),
        ..context
    };
    let problem = broken.compile("anything").unwrap_err();
    assert!(
        problem.starts_with("The system message doesn't compile."),
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

    for (variable, meaning) in VARIABLES {
        // The loop's page text has an ellipsis for its body.
        let template = variable.replace('…', "{{ item.url }}");
        let compiled = context.compile(&template);
        assert!(compiled.is_ok(), "{variable}: {compiled:?}");
        assert!(meaning.ends_with('.'), "{variable}");
    }
}
