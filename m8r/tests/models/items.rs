use std::fs;

use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        items::{self, Forge, Params, Remote},
        tool_errors,
    },
};
use serial_test::serial;

use crate::support::{Sandbox, GH, GLAB};

fn params(url: &str, priority: &str) -> Params {
    Params {
        url: url.to_string(),
        priority: priority.to_string(),
        note: String::new(),
    }
}

async fn add(db: &DatabaseConnection, url: &str) -> items::Model {
    items::Model::create(db, &params(url, "medium"))
        .await
        .unwrap()
}

async fn urls(db: &DatabaseConnection) -> Vec<String> {
    let mut urls: Vec<_> = items::Model::list(db)
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.url)
        .collect();
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
}

#[test]
fn reads_gitlab_urls() {
    let mr = Forge::parse("https://git.rgnets.com/rgnets/rxg/-/merge_requests/4521").unwrap();
    assert_eq!(mr.tool, "glab");
    assert_eq!(mr.host, "git.rgnets.com");
    assert_eq!(mr.endpoint, "projects/rgnets%2Frxg/merge_requests/4521");
    assert_eq!(mr.label, "rgnets/rxg!4521");

    let issue = Forge::parse("https://git.example.com/g/sub/p/-/issues/3#note_9").unwrap();
    assert_eq!(issue.endpoint, "projects/g%2Fsub%2Fp/issues/3");
    assert_eq!(issue.label, "g/sub/p#3");

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
    assert!(params("https://example.com/", "high").problem().is_some());
    assert!(params("https://github.com/o/r/issues/1", "low")
        .problem()
        .is_some());
    assert!(params("https://github.com/o/r/issues/1", "high")
        .problem()
        .is_none());
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
async fn adding_an_item_twice_updates_it() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let item = add(db, " https://github.com/o/r/issues/1 ").await;
    assert_eq!(item.url, "https://github.com/o/r/issues/1");
    assert_eq!(item.label(), "o/r#1");

    let mut again = params("https://github.com/o/r/issues/1", "high");
    again.note = "Now it's urgent".to_string();
    let same = items::Model::create(db, &again).await.unwrap();

    assert_eq!(same.id, item.id);
    assert_eq!(same.priority, "high");
    assert_eq!(same.note, "Now it's urgent");
    assert_eq!(urls(db).await.len(), 1);
}

#[tokio::test]
#[serial]
async fn the_forge_gives_titles_and_closed_items_leave_the_list() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("items-refresh");
    sandbox.tool("gh", GH);
    sandbox.tool("glab", GLAB);
    add(db, "https://github.com/o/r/issues/1").await;
    add(db, "https://github.com/o/r/pull/2").await;
    add(db, "https://git.example.com/g/s/p/-/merge_requests/7").await;
    add(db, "https://git.example.com/g/s/p/-/issues/8").await;

    items::Model::refresh(db, &sandbox.shell()).await.unwrap();

    assert_eq!(
        urls(db).await,
        [
            "https://git.example.com/g/s/p/-/issues/8",
            "https://github.com/o/r/issues/1",
        ]
    );
    let titles: Vec<_> = items::Model::list(db)
        .await
        .unwrap()
        .into_iter()
        .map(|item| (item.title, item.checked_at.is_some()))
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
    let sandbox = Sandbox::new("items-missing");
    add(db, "https://github.com/o/r/issues/1").await;
    add(db, "https://git.example.com/g/s/p/-/issues/8").await;

    // Neither tool is installed. The items stay, unchecked.
    items::Model::refresh(db, &sandbox.shell()).await.unwrap();
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
    items::Model::refresh(db, &sandbox.shell()).await.unwrap();
    let left = errors(db).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].0, "glab");

    // Nothing is on GitLab any more, so nothing needs glab.
    let gitlab = items::Model::list(db)
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.url.contains("git.example.com"))
        .unwrap();
    gitlab.delete(db).await.unwrap();
    items::Model::refresh(db, &sandbox.shell()).await.unwrap();
    assert!(errors(db).await.is_empty());
}

#[tokio::test]
#[serial]
async fn a_failed_login_is_an_error_until_it_is_fixed() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("items-login");
    sandbox.tool("gh", GH);
    add(db, "https://github.com/o/r/issues/1").await;
    let logged_out = sandbox.path("bin/logged-out");
    fs::write(&logged_out, "").unwrap();

    items::Model::refresh(db, &sandbox.shell()).await.unwrap();
    assert_eq!(
        errors(db).await,
        [(
            "gh".to_string(),
            "github.com".to_string(),
            "o/r#1: gh: To get started with GitHub CLI, please run: gh auth login".to_string(),
        )]
    );

    fs::remove_file(&logged_out).unwrap();
    items::Model::refresh(db, &sandbox.shell()).await.unwrap();
    assert!(errors(db).await.is_empty());
    assert_eq!(
        items::Model::list(db).await.unwrap()[0].title,
        "An open one"
    );
}

#[tokio::test]
#[serial]
async fn one_item_that_fails_does_not_stop_the_others() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("items-one-fails");
    sandbox.tool("gh", GH);
    add(db, "https://github.com/o/r/issues/3").await;
    add(db, "https://github.com/o/r/issues/2").await;

    items::Model::refresh(db, &sandbox.shell()).await.unwrap();

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
async fn a_fresh_item_is_not_asked_about_again_at_once() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("items-fresh");
    sandbox.tool("gh", GH);
    add(db, "https://github.com/o/r/issues/1").await;
    items::Model::refresh(db, &sandbox.shell()).await.unwrap();
    let checked_at = items::Model::list(db).await.unwrap()[0].checked_at;

    // Without gh, a second look would fail. It doesn't look: the item is fresh.
    sandbox.remove_tool("gh");
    items::Model::refresh(db, &sandbox.shell()).await.unwrap();

    assert!(errors(db).await.is_empty());
    assert_eq!(
        items::Model::list(db).await.unwrap()[0].checked_at,
        checked_at
    );
}
