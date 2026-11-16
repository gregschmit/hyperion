use std::fs;

use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        categories, tool_errors,
        watchlist_items::{self, Changes, Forge, Params, Remote},
    },
};
use serial_test::serial;

use crate::support::{Sandbox, GH, GLAB};

/// The category that every m8r starts with for what's at `url`.
fn category_of(url: &str) -> &'static str {
    if url.contains("/pull/") || url.contains("/merge_requests/") {
        "Pull/Merge Request"
    } else {
        "Issue"
    }
}

fn params(url: &str, priority: &str) -> Params {
    Params {
        url: url.to_string(),
        category: category_of(url).to_string(),
        priority: priority.to_string(),
        ..Params::default()
    }
}

async fn watch(db: &DatabaseConnection, url: &str) -> watchlist_items::Model {
    let listed = watchlist_items::Model::create(db, &params(url, "medium")).await;
    listed.unwrap().unwrap().0
}

/// Why `params` can't be saved.
async fn problem(db: &DatabaseConnection, params: &Params) -> String {
    let listed = watchlist_items::Model::create(db, params).await;
    listed.unwrap().unwrap_err()
}

async fn items(db: &DatabaseConnection) -> Vec<watchlist_items::Model> {
    let listed = watchlist_items::Model::list(db, None).await.unwrap();
    listed.into_iter().map(|(item, _)| item).collect()
}

async fn urls(db: &DatabaseConnection) -> Vec<String> {
    let mut urls: Vec<_> = items(db).await.into_iter().map(|item| item.url).collect();
    urls.sort();
    urls
}

async fn errors(db: &DatabaseConnection) -> Vec<(String, String, String)> {
    tool_errors::Model::list(db)
        .await
        .unwrap()
        .into_iter()
        .map(|error| (error.tool, error.host, error.message))
        .collect()
}

#[test]
fn reads_github_urls() {
    let issue = Forge::parse("https://github.com/gregschmit/hyperion/issues/12").unwrap();
    assert_eq!(issue.tool, "gh");
    assert_eq!(issue.host, "github.com");
    assert_eq!(issue.endpoint, "repos/gregschmit/hyperion/issues/12");
    assert_eq!(issue.label, "gregschmit/hyperion#12");
    assert!(!issue.reviewable);
    assert_eq!(
        issue.command(),
        [
            "gh",
            "api",
            "--hostname",
            "github.com",
            "repos/gregschmit/hyperion/issues/12"
        ]
    );

    // A PR is an issue to GitHub's API. What comes after the number doesn't matter.
    let pr = Forge::parse(" https://github.com/o/r/pull/7/files?diff=split#top ").unwrap();
    assert_eq!(pr.endpoint, "repos/o/r/issues/7");
    assert_eq!(pr.label, "o/r#7");
    assert!(pr.reviewable);
}

#[test]
fn reads_gitlab_urls() {
    let mr = Forge::parse("https://git.rgnets.com/rgnets/rxg/-/merge_requests/4521").unwrap();
    assert_eq!(mr.tool, "glab");
    assert_eq!(mr.host, "git.rgnets.com");
    assert_eq!(mr.endpoint, "projects/rgnets%2Frxg/merge_requests/4521");
    assert_eq!(mr.label, "rgnets/rxg!4521");
    assert!(mr.reviewable);

    let issue = Forge::parse("https://git.example.com/g/sub/p/-/issues/3#note_9").unwrap();
    assert_eq!(issue.endpoint, "projects/g%2Fsub%2Fp/issues/3");
    assert_eq!(issue.label, "g/sub/p#3");
    assert!(!issue.reviewable);

    // Newer GitLab calls an issue a work item, and older URLs have no "/-/".
    let work_item = Forge::parse("https://git.example.com/g/p/-/work_items/3").unwrap();
    assert_eq!(work_item.endpoint, "projects/g%2Fp/issues/3");
    let old = Forge::parse("http://git.example.com:8443/g/p/merge_requests/3").unwrap();
    assert_eq!(old.host, "git.example.com:8443");
    assert_eq!(old.endpoint, "projects/g%2Fp/merge_requests/3");
}

#[test]
fn refuses_what_is_not_an_issue_or_a_pr() {
    for url in [
        "",
        "github.com/o/r/issues/1",
        "ftp://github.com/o/r/issues/1",
        "https://github.com/o/r",
        "https://github.com/o/r/issues/",
        "https://github.com/o/r/issues/new",
        "https://github.com/o/r/tree/main",
        "https://git.example.com/p/-/issues/3",
        "https://git.example.com/g/p/-/pipelines/3",
        "https://git.example.com/g/p;rm -rf/-/issues/3",
        "https://git.example.com/g/$(id)/-/issues/3",
    ] {
        assert_eq!(Forge::parse(url), None, "{url}");
    }
}

#[tokio::test]
#[serial]
async fn refuses_an_item_that_it_cannot_keep() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let issue = "https://github.com/o/r/issues/1";

    let not_one = problem(db, &params("https://example.com/", "high")).await;
    assert!(not_one.starts_with("That isn't the URL"), "{not_one}");
    let priority = problem(db, &params(issue, "low")).await;
    assert_eq!(priority, "The priority is medium or high.");

    // Only a PR/MR can have its review requested.
    let review = |url: &str| Params {
        review_requested: true,
        ..params(url, "medium")
    };
    for url in [issue, "https://git.example.com/g/p/-/issues/1"] {
        let refused = problem(db, &review(url)).await;
        assert!(refused.starts_with("Only a PR/MR"), "{url}: {refused}");
    }
    for url in [
        "https://github.com/o/r/pull/1",
        "https://git.example.com/g/p/-/merge_requests/1",
    ] {
        let listed = watchlist_items::Model::create(db, &review(url)).await;
        assert!(listed.unwrap().is_ok(), "{url}");
    }

    // The answer to a category that isn't there names the ones that are.
    let nowhere = Params {
        category: "Epic".to_string(),
        ..params(issue, "medium")
    };
    assert_eq!(
        problem(db, &nowhere).await,
        "There's no category \"Epic\". The categories are: Issue, Pull/Merge Request."
    );
    let none = Params {
        category: String::new(),
        ..params(issue, "medium")
    };
    assert_eq!(
        problem(db, &none).await,
        "Choose a category. The categories are: Issue, Pull/Merge Request."
    );
    assert_eq!(urls(db).await.len(), 2);
}

#[tokio::test]
#[serial]
async fn the_list_goes_by_category_and_the_urgent_items_come_first() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    watch(db, "https://github.com/o/r/pull/1").await;
    watch(db, "https://github.com/o/r/issues/2").await;
    let urgent = params("https://github.com/o/r/issues/3", "high");
    watchlist_items::Model::create(db, &urgent)
        .await
        .unwrap()
        .unwrap();

    let all: Vec<_> = watchlist_items::Model::list(db, None)
        .await
        .unwrap()
        .into_iter()
        .map(|(item, category)| (item.label(), category.name))
        .collect();
    assert_eq!(
        all,
        [
            ("o/r#3".to_string(), "Issue".to_string()),
            ("o/r#2".to_string(), "Issue".to_string()),
            ("o/r#1".to_string(), "Pull/Merge Request".to_string()),
        ]
    );

    let prs = categories::Model::named(db, "Pull/Merge Request")
        .await
        .unwrap()
        .unwrap();
    let only = watchlist_items::Model::list(db, Some(prs.id))
        .await
        .unwrap();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].0.label(), "o/r#1");
}

#[tokio::test]
#[serial]
async fn a_change_keeps_what_it_does_not_name() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let first = Params {
        note: "Ships on Friday".to_string(),
        review_requested: true,
        ..params("https://github.com/o/r/pull/1", "high")
    };
    let (item, _) = watchlist_items::Model::create(db, &first)
        .await
        .unwrap()
        .unwrap();

    let reviewed = Changes {
        review_requested: Some(false),
        ..Changes::default()
    };
    let (changed, category) = watchlist_items::Model::change(db, item.id, reviewed)
        .await
        .unwrap()
        .unwrap();

    assert!(!changed.review_requested);
    assert_eq!(
        (changed.priority.as_str(), changed.note.as_str()),
        ("high", "Ships on Friday")
    );
    assert_eq!(category.name, "Pull/Merge Request");

    // A change that can't be made changes nothing.
    let moved = Changes {
        category: Some("Epic".to_string()),
        note: Some("Lost".to_string()),
        ..Changes::default()
    };
    let refused = watchlist_items::Model::change(db, item.id, moved).await;
    assert!(refused.unwrap().is_err());
    assert_eq!(items(db).await[0].note, "Ships on Friday");

    watchlist_items::Model::remove(db, item.id).await.unwrap();
    assert!(items(db).await.is_empty());
    let gone = watchlist_items::Model::remove(db, item.id).await;
    assert!(matches!(gone, Err(ModelError::EntityNotFound)));
}

#[test]
fn reads_what_the_forge_answers() {
    let open = Remote::Open {
        title: "A title".to_string(),
    };
    assert_eq!(
        Forge::read(r#"{"title": "A title", "state": "open"}"#),
        Some(open)
    );
    assert!(matches!(
        Forge::read(r#"{"title": "T", "state": "opened"}"#),
        Some(Remote::Open { .. })
    ));
    assert_eq!(
        Forge::read(r#"{"title": "T", "state": "closed"}"#),
        Some(Remote::Closed)
    );
    assert_eq!(
        Forge::read(r#"{"title": "T", "state": "merged"}"#),
        Some(Remote::Closed)
    );
    // A login shell may say something first, and a newline comes after.
    assert_eq!(
        Forge::read("Welcome back!\n{\"title\": \"T\", \"state\": \"closed\"}\n"),
        Some(Remote::Closed)
    );
    assert_eq!(Forge::read("no json here"), None);
    assert_eq!(Forge::read(r#"{"message": "404 Not Found"}"#), None);
}

#[tokio::test]
#[serial]
async fn adding_something_twice_updates_the_item() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let first = watch(db, " https://github.com/o/r/pull/1 ").await;
    assert_eq!(first.url, "https://github.com/o/r/pull/1");
    assert_eq!(first.label(), "o/r#1");
    assert!(first.reviewable());
    assert!(!first.review_requested);

    let again = Params {
        note: "Now it's urgent".to_string(),
        review_requested: true,
        ..params("https://github.com/o/r/pull/1", "high")
    };
    let (same, _) = watchlist_items::Model::create(db, &again)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(same.id, first.id);
    assert_eq!(same.priority, "high");
    assert_eq!(same.note, "Now it's urgent");
    assert!(same.review_requested);
    assert_eq!(urls(db).await.len(), 1);
}

#[tokio::test]
#[serial]
async fn the_forge_gives_titles_and_what_closed_leaves_the_watchlist() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("watches-refresh");
    sandbox.tool("gh", GH);
    sandbox.tool("glab", GLAB);
    watch(db, "https://github.com/o/r/issues/1").await;
    watch(db, "https://github.com/o/r/pull/2").await;
    watch(db, "https://git.example.com/g/s/p/-/merge_requests/7").await;
    watch(db, "https://git.example.com/g/s/p/-/issues/8").await;

    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();

    assert_eq!(
        urls(db).await,
        [
            "https://git.example.com/g/s/p/-/issues/8",
            "https://github.com/o/r/issues/1",
        ]
    );
    let titles: Vec<_> = items(db)
        .await
        .into_iter()
        .map(|watch| (watch.title, watch.checked_at.is_some()))
        .collect();
    assert_eq!(
        titles,
        [
            ("An open one".to_string(), true),
            ("An opened one".to_string(), true)
        ]
    );
    assert!(errors(db).await.is_empty());
}

#[tokio::test]
#[serial]
async fn a_missing_tool_is_an_error_until_it_shows_up() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("watches-missing");
    watch(db, "https://github.com/o/r/issues/1").await;
    watch(db, "https://git.example.com/g/s/p/-/issues/8").await;

    // Neither tool is installed. The watches stay, unchecked.
    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();
    let missing = errors(db).await;
    assert_eq!(missing.len(), 2);
    assert_eq!((missing[0].0.as_str(), missing[0].1.as_str()), ("gh", ""));
    assert_eq!((missing[1].0.as_str(), missing[1].1.as_str()), ("glab", ""));
    assert!(
        missing[0].2.starts_with("It isn't installed"),
        "{}",
        missing[0].2
    );
    assert_eq!(urls(db).await.len(), 2);

    // gh shows up: its error goes at the next look, and glab's stays.
    sandbox.tool("gh", GH);
    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();
    let left = errors(db).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].0, "glab");

    // Nothing on GitLab is watched any more, so nothing needs glab.
    let gitlab = items(db)
        .await
        .into_iter()
        .find(|watch| watch.url.contains("git.example.com"))
        .unwrap();
    gitlab.delete(db).await.unwrap();
    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();
    assert!(errors(db).await.is_empty());
}

#[tokio::test]
#[serial]
async fn a_failed_login_is_an_error_until_it_is_fixed() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("watches-login");
    sandbox.tool("gh", GH);
    watch(db, "https://github.com/o/r/issues/1").await;
    let logged_out = sandbox.path("bin/logged-out");
    fs::write(&logged_out, "").unwrap();

    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();
    assert_eq!(
        errors(db).await,
        [(
            "gh".to_string(),
            "github.com".to_string(),
            "o/r#1: gh: To get started with GitHub CLI, please run: gh auth login".to_string(),
        )]
    );

    fs::remove_file(&logged_out).unwrap();
    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();
    assert!(errors(db).await.is_empty());
    assert_eq!(items(db).await[0].title, "An open one");
}

#[tokio::test]
#[serial]
async fn one_watch_that_fails_does_not_stop_the_others() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("watches-one-fails");
    sandbox.tool("gh", GH);
    watch(db, "https://github.com/o/r/issues/3").await;
    watch(db, "https://github.com/o/r/issues/2").await;

    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();

    // 3 doesn't exist, and says so. 2 is closed, and went.
    assert_eq!(urls(db).await, ["https://github.com/o/r/issues/3"]);
    assert_eq!(
        errors(db).await,
        [(
            "gh".to_string(),
            "github.com".to_string(),
            "o/r#3: gh: Not Found (HTTP 404)".to_string(),
        )]
    );
}

#[tokio::test]
#[serial]
async fn a_fresh_watch_is_not_asked_about_again_at_once() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("watches-fresh");
    sandbox.tool("gh", GH);
    watch(db, "https://github.com/o/r/issues/1").await;
    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();
    let checked_at = items(db).await[0].checked_at;

    // Without gh, a second look would fail. It doesn't look: the watch is fresh.
    sandbox.remove_tool("gh");
    watchlist_items::Model::refresh(db, &sandbox.shell())
        .await
        .unwrap();

    assert!(errors(db).await.is_empty());
    assert_eq!(items(db).await[0].checked_at, checked_at);
}
