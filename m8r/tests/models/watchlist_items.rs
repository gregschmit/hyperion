use std::fs;

use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        errors,
        watchlist_items::{self, Category, Changes, Filter, Forge, Params, Remote},
    },
};
use serial_test::serial;

use crate::support::{self, Sandbox, GH, GLAB};

fn params(url: &str, priority: &str) -> Params {
    Params {
        url: url.to_string(),
        priority: priority.to_string(),
        ..Params::default()
    }
}

/// Put what's at `url` on the watchlist. Its project's repo is there first.
async fn watch(db: &DatabaseConnection, url: &str) -> watchlist_items::Model {
    let project = Forge::parse(url).unwrap().project;
    let known = m8r::models::repos::Model::of_project(db, &project).await;
    if known.unwrap().is_none() {
        support::repo(db, &project.replace("https://", "/clones/"), &project).await;
    }
    let listed = watchlist_items::Model::create(db, &params(url, "medium")).await;
    listed.unwrap().unwrap().0
}

/// Why `params` can't be saved.
async fn problem(db: &DatabaseConnection, params: &Params) -> String {
    let listed = watchlist_items::Model::create(db, params).await;
    listed.unwrap().unwrap_err()
}

async fn items(db: &DatabaseConnection) -> Vec<watchlist_items::Model> {
    let listed = watchlist_items::Model::list(db, Filter::default())
        .await
        .unwrap();
    listed.into_iter().map(|(item, _)| item).collect()
}

async fn urls(db: &DatabaseConnection) -> Vec<String> {
    let mut urls: Vec<_> = items(db).await.into_iter().map(|item| item.url).collect();
    urls.sort();
    urls
}

/// The open errors: the tool, its host (blank when it's about the tool itself), and what's
/// wrong.
async fn errors(db: &DatabaseConnection) -> Vec<(String, String, String)> {
    errors::Model::open(db)
        .await
        .unwrap()
        .into_iter()
        .map(|error| {
            let (tool, host) = error
                .subject
                .split_once(" on ")
                .unwrap_or((&error.subject, ""));
            (tool.to_string(), host.to_string(), error.message.clone())
        })
        .collect()
}

#[test]
fn reads_github_urls() {
    let issue = Forge::parse("https://github.com/gregschmit/hyperion/issues/12").unwrap();
    assert_eq!(issue.tool, "gh");
    assert_eq!(issue.host, "github.com");
    assert_eq!(issue.endpoint, "repos/gregschmit/hyperion/issues/12");
    assert_eq!(issue.label, "gregschmit/hyperion#12");
    assert_eq!(issue.number, 12);
    // The URL says which repo the item is in, and what it is.
    assert_eq!(issue.project, "https://github.com/gregschmit/hyperion");
    assert_eq!(issue.category, Category::Issues);
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

    // A PR is an issue to GitHub's API. What comes after the number doesn't matter, and
    // neither does the case of the host.
    let pr = Forge::parse(" https://GitHub.com/o/r/pull/7/files?diff=split#top ").unwrap();
    assert_eq!(pr.endpoint, "repos/o/r/issues/7");
    assert_eq!(pr.label, "o/r#7");
    assert_eq!(pr.project, "https://github.com/o/r");
    assert_eq!(pr.category, Category::Requests);
}

#[test]
fn reads_gitlab_urls() {
    let mr = Forge::parse("https://git.rgnets.com/rgnets/rxg/-/merge_requests/4521").unwrap();
    assert_eq!(mr.tool, "glab");
    assert_eq!(mr.host, "git.rgnets.com");
    assert_eq!(mr.endpoint, "projects/rgnets%2Frxg/merge_requests/4521");
    assert_eq!(mr.label, "rgnets/rxg!4521");
    assert_eq!(mr.project, "https://git.rgnets.com/rgnets/rxg");
    assert_eq!(mr.category, Category::Requests);

    // A project can be in a group in a group.
    let issue = Forge::parse("https://git.example.com/g/sub/p/-/issues/3#note_9").unwrap();
    assert_eq!(issue.endpoint, "projects/g%2Fsub%2Fp/issues/3");
    assert_eq!(issue.label, "g/sub/p#3");
    assert_eq!(issue.project, "https://git.example.com/g/sub/p");
    assert_eq!(issue.category, Category::Issues);

    // Newer GitLab calls an issue a work item, and older URLs have no "/-/".
    let work_item = Forge::parse("https://git.example.com/g/p/-/work_items/3").unwrap();
    assert_eq!(work_item.endpoint, "projects/g%2Fp/issues/3");
    assert_eq!(work_item.category, Category::Issues);
    let old = Forge::parse("http://git.example.com:8443/g/p/merge_requests/3").unwrap();
    assert_eq!(old.host, "git.example.com:8443");
    assert_eq!(old.endpoint, "projects/g%2Fp/merge_requests/3");
    // A project's URL is the same, however its item's URL starts.
    assert_eq!(old.project, "https://git.example.com:8443/g/p");
}

#[test]
fn the_categories_have_their_names() {
    assert_eq!(
        Category::ALL.map(Category::as_str),
        ["issues", "pull/merge requests"]
    );
    assert_eq!(Category::parse("issues"), Some(Category::Issues));
    assert_eq!(Category::parse("Issue"), None);
    // No name is no category, which a list takes for all of them.
    assert_eq!(Category::named(None), Ok(None));
    assert_eq!(Category::named(Some(" ")), Ok(None));
    assert_eq!(
        Category::named(Some("pull/merge requests")),
        Ok(Some(Category::Requests))
    );
    assert_eq!(
        Category::named(Some("Epic")),
        Err(
            "There's no category \"Epic\". The categories are: issues, pull/merge requests."
                .to_string()
        )
    );
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
    assert_eq!(priority, "The priority is medium, high or urgent.");

    // An item is in a repo, and m8r has none for this one's project yet.
    assert_eq!(
        problem(db, &params(issue, "medium")).await,
        "m8r has no repo for https://github.com/o/r. Add that repo first: an item is in one."
    );
    assert!(urls(db).await.is_empty());
    // A repo of another project doesn't do, and neither does one with no project.
    support::repo(db, "/clones/other", "https://github.com/o/other").await;
    support::repo(db, "/clones/local", "").await;
    assert!(problem(db, &params(issue, "medium"))
        .await
        .starts_with("m8r has no repo"));

    // With its repo, it goes on the list. The URL says what it is, and the forge reads the
    // project's path without case. Nobody said how much it matters, so it's medium: with no
    // priority, and with a blank one.
    let repo = support::repo(db, "/clones/r", "https://github.com/O/R").await;
    assert_eq!(Params::default().priority, "medium");
    for unsaid in [Params::default().priority, String::new()] {
        let listed = watchlist_items::Model::create(db, &params(issue, &unsaid)).await;
        let (item, its_repo) = listed.unwrap().unwrap();
        assert_eq!((its_repo.id, item.repo_id), (repo.id, repo.id));
        assert_eq!(item.category, "issues");
        assert_eq!(item.priority, "medium");
    }

    // Someone can ask you for any item: for an issue too.
    for url in [issue, "https://github.com/o/r/pull/2"] {
        let asked = Params {
            requested: true,
            ..params(url, "medium")
        };
        let listed = watchlist_items::Model::create(db, &asked).await;
        assert!(listed.unwrap().unwrap().0.requested, "{url}");
    }
    assert_eq!(items(db).await[1].category, "pull/merge requests");
}

#[tokio::test]
#[serial]
async fn the_list_has_the_most_pressing_items_first() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let one = support::repo(db, "/clones/a", "https://github.com/o/r").await;
    let two = support::repo(db, "/clones/b", "https://git.example.com/g/p").await;
    watch(db, "https://git.example.com/g/p/-/issues/9").await;
    watch(db, "https://github.com/o/r/issues/10").await;
    watch(db, "https://github.com/o/r/pull/1").await;
    watch(db, "https://github.com/o/r/issues/2").await;
    let high = params("https://github.com/o/r/issues/3", "high");
    let asked = Params {
        requested: true,
        ..params("https://github.com/o/r/pull/4", "medium")
    };
    let urgent = params("https://github.com/o/r/issues/5", "urgent");
    for params in [high, asked, urgent] {
        let listed = watchlist_items::Model::create(db, &params).await;
        listed.unwrap().unwrap();
    }
    let labels = |listed: Vec<watchlist_items::Listed>| -> Vec<String> {
        listed.iter().map(|(item, _)| item.label()).collect()
    };

    // What's urgent, then what's high, then the rest. Of the rest, first what someone asked
    // for. Then repo by repo, and in each, the issues before the PRs/MRs, by their numbers:
    // 2 is before 10, whichever was added first.
    let all = watchlist_items::Model::list(db, Filter::default()).await;
    let all = all.unwrap();
    assert_eq!(all[0].1.id, one.id);
    assert_eq!(all[6].1.id, two.id);
    assert_eq!(
        labels(all),
        ["o/r#5", "o/r#3", "o/r#4", "o/r#2", "o/r#10", "o/r#1", "g/p#9"]
    );

    // A list can be a category's, a repo's, or both.
    let requests = Filter {
        category: Some(Category::Requests),
        ..Filter::default()
    };
    let only = watchlist_items::Model::list(db, requests).await.unwrap();
    assert_eq!(labels(only), ["o/r#4", "o/r#1"]);
    let of_two = Filter {
        repo_id: Some(two.id),
        ..Filter::default()
    };
    let only = watchlist_items::Model::list(db, of_two).await.unwrap();
    assert_eq!(labels(only), ["g/p#9"]);
    let neither = Filter {
        repo_id: Some(two.id),
        category: Some(Category::Requests),
    };
    let none = watchlist_items::Model::list(db, neither).await.unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
#[serial]
async fn a_run_gets_its_repos_items_and_the_first_of_them() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let one = support::repo(db, "/clones/a", "https://github.com/o/r").await;
    let two = support::repo(db, "/clones/b", "https://git.example.com/g/p").await;
    for (url, priority, requested) in [
        ("https://github.com/o/r/issues/1", "medium", false),
        ("https://github.com/o/r/pull/2", "medium", false),
        ("https://github.com/o/r/issues/3", "high", false),
        ("https://github.com/o/r/pull/4", "medium", true),
        ("https://git.example.com/g/p/-/issues/5", "high", true),
        ("https://github.com/o/r/pull/6", "urgent", false),
        ("https://github.com/o/r/issues/7", "high", true),
    ] {
        let asked = Params {
            requested,
            ..params(url, priority)
        };
        let listed = watchlist_items::Model::create(db, &asked).await;
        listed.unwrap().unwrap();
    }
    let got = |repo_id: i64, category: Option<Category>, max_items: u64| async move {
        let items = watchlist_items::Model::of_repo(db, repo_id, category, max_items).await;
        let ids: Vec<i64> = items.unwrap().iter().map(|item| item.id).collect();
        ids
    };

    // Its own repo's, and no other's. The requested ones come first, and the more pressing
    // of them before the other. Then the urgent one, the high one, and the rest.
    assert_eq!(got(one.id, None, 0).await, [7, 4, 6, 3, 1, 2]);
    assert_eq!(got(two.id, None, 0).await, [5]);
    assert_eq!(got(one.id, Some(Category::Issues), 0).await, [7, 3, 1]);
    assert_eq!(got(one.id, Some(Category::Requests), 0).await, [4, 6, 2]);
    // A cutoff keeps the first ones, so what someone asked for is in.
    assert_eq!(got(one.id, None, 3).await, [7, 4, 6]);
    assert_eq!(got(one.id, Some(Category::Requests), 1).await, [4]);
    assert_eq!(got(one.id, None, 9).await, [7, 4, 6, 3, 1, 2]);
}

#[tokio::test]
#[serial]
async fn a_change_keeps_what_it_does_not_name() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    support::repo(db, "/clones/r", "https://github.com/o/r").await;
    let first = Params {
        note: "Ships on Friday".to_string(),
        requested: true,
        ..params("https://github.com/o/r/pull/1", "high")
    };
    let (item, _) = watchlist_items::Model::create(db, &first)
        .await
        .unwrap()
        .unwrap();

    let reviewed = Changes {
        requested: Some(false),
        ..Changes::default()
    };
    let (changed, repo) = watchlist_items::Model::change(db, item.id, reviewed)
        .await
        .unwrap()
        .unwrap();

    assert!(!changed.requested);
    assert_eq!(
        (changed.priority.as_str(), changed.note.as_str()),
        ("high", "Ships on Friday")
    );
    // Its URL says where it is and what it is, so those stay.
    assert_eq!(
        (changed.category.as_str(), repo.path.as_str()),
        ("pull/merge requests", "/clones/r")
    );

    // A change that can't be made changes nothing.
    let lowered = Changes {
        priority: Some("low".to_string()),
        note: Some("Lost".to_string()),
        ..Changes::default()
    };
    let refused = watchlist_items::Model::change(db, item.id, lowered).await;
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
    assert!(!first.requested);

    let again = Params {
        note: "Now it's urgent".to_string(),
        requested: true,
        ..params("https://github.com/o/r/pull/1", "high")
    };
    let (same, _) = watchlist_items::Model::create(db, &again)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(same.id, first.id);
    assert_eq!(same.priority, "high");
    assert_eq!(same.note, "Now it's urgent");
    assert!(same.requested);
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
            ("An opened one".to_string(), true),
            ("An open one".to_string(), true)
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
    assert_eq!(
        missing[0].2,
        "m8r asks the forge about the watchlist with gh, and the login shell of m8r's user \
         finds no such command."
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
    // The error is over, and the list keeps it.
    let kept = errors::Model::list(db, 10).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].subject, "gh on github.com");
    assert!(kept[0].cleared_at.is_some());
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
