//! The repos that m8r works in.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        errors,
        repos::{self, Changes, Params},
        runs,
    },
};
use serial_test::serial;

use crate::support::{self, Sandbox};

fn params(path: &Path, url: &str) -> Params {
    Params {
        path: path.to_string_lossy().into_owned(),
        url: url.to_string(),
    }
}

/// A clone in the sandbox with this `origin`, or with none.
fn clone(sandbox: &Sandbox, name: &str, origin: Option<&str>) -> PathBuf {
    let dir = sandbox.another_repo(name);
    let git = |args: &[&str]| {
        let done = Command::new("git").arg("-C").arg(&dir).args(args).output();
        assert!(done.unwrap().status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    if let Some(origin) = origin {
        git(&["remote", "add", "origin", origin]);
    }
    dir
}

/// What the open errors are about, and what they say.
async fn open(db: &DatabaseConnection) -> Vec<(String, String)> {
    let open = errors::Model::open(db).await.unwrap();
    open.into_iter()
        .map(|error| (error.subject, error.message))
        .collect()
}

#[test]
fn reads_a_project_from_its_url_or_from_a_remote() {
    let project = Some("https://git.example.com/group/project".to_string());
    for text in [
        "https://git.example.com/group/project",
        " https://git.example.com/group/project/ ",
        "https://git.example.com/group/project.git",
        "http://Git.Example.com/group/project",
        "https://token@git.example.com/group/project.git",
        "git@git.example.com:group/project.git",
        "ssh://git@git.example.com/group/project.git",
        "ssh://git@git.example.com:2222/group/project.git",
        "git.example.com/group/project",
    ] {
        assert_eq!(repos::project_url(text), project, "{text}");
    }
    // A project can be in a group in a group, and a browser's port is part of the host.
    assert_eq!(
        repos::project_url("git@git.example.com:g/sub/p.git").unwrap(),
        "https://git.example.com/g/sub/p"
    );
    assert_eq!(
        repos::project_url("https://git.example.com:8443/g/p").unwrap(),
        "https://git.example.com:8443/g/p"
    );
    assert_eq!(
        repos::project_url("git@github.com:gregschmit/hyperion.git").unwrap(),
        "https://github.com/gregschmit/hyperion"
    );
    for text in [
        "",
        "project",
        "https://git.example.com",
        "https://git.example.com/project",
        "/home/m8r/git/project",
        "git@git.example.com:group/pro ject.git",
        "https://git.example.com/group/$(id)",
    ] {
        assert_eq!(repos::project_url(text), None, "{text}");
    }
}

#[test]
fn a_clones_path_is_from_the_home_of_m8rs_user() {
    let home = Path::new("/home/m8r");
    for path in [
        "~/git/group/project",
        "git/group/project",
        " ~/git/group/project ",
    ] {
        assert_eq!(
            repos::from_home(path, home),
            Path::new("/home/m8r/git/group/project"),
            "{path}"
        );
    }
    assert_eq!(repos::from_home("~", home), home);
    // A full path stays, and a `~` that isn't the home's stays a name.
    assert_eq!(
        repos::from_home("/srv/project", home),
        Path::new("/srv/project")
    );
    assert_eq!(
        repos::from_home("~other/x", home),
        Path::new("/home/m8r/~other/x")
    );
    // The home is the one of the user that m8r runs as.
    let own = std::env::home_dir().unwrap();
    assert_eq!(repos::dir_of("~/git/project"), own.join("git/project"));
}

#[tokio::test]
#[serial]
async fn a_repo_is_a_clone_that_is_there_and_a_project_of_its_own() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("repos-create");
    let shell = sandbox.shell();
    let (one, two) = (sandbox.repo(), sandbox.another_repo("two"));
    let problem = |params: Params| {
        let shell = shell.clone();
        async move {
            let made = repos::Model::create(db, &shell, &params).await;
            made.unwrap().unwrap_err()
        }
    };

    let nowhere = problem(params(Path::new(" "), "")).await;
    assert!(nowhere.starts_with("Give the repo a path"), "{nowhere}");
    let missing = problem(params(&sandbox.path("no-such-clone"), "")).await;
    assert!(missing.contains("isn't a directory there"), "{missing}");
    let no_project = problem(params(&one, "https://git.example.com/project")).await;
    assert!(
        no_project.starts_with("That isn't the URL of a project"),
        "{no_project}"
    );
    assert!(repos::Model::list(db).await.unwrap().is_empty());

    // The project is kept as m8r reads it, whatever way it's written.
    let made = repos::Model::create(db, &shell, &params(&one, "git@git.example.com:g/p.git")).await;
    let made = made.unwrap().unwrap();
    assert_eq!(made.url, "https://git.example.com/g/p");
    assert_eq!(made.dir(), one);

    // A clone is one repo, however its path is written, and so is a project.
    let again = problem(params(&one.join("../repo"), "")).await;
    assert_eq!(again, format!("m8r already has that clone: {}.", made.path));
    let same = problem(params(&two, "https://Git.Example.com/G/P")).await;
    assert_eq!(
        same,
        format!(
            "https://git.example.com/G/P is the project of the repo {}, and a project has one \
             repo.",
            made.path
        )
    );

    // A clone with no project is a repo too: a schedule can run in it.
    let local = repos::Model::create(db, &shell, &params(&two, "")).await;
    let local = local.unwrap().unwrap();
    assert_eq!(local.url, "");
    let found = repos::Model::of_project(db, "https://git.example.com/g/p").await;
    assert_eq!(found.unwrap().unwrap().id, made.id);
    // No project is nobody's project.
    assert!(repos::Model::of_project(db, "").await.unwrap().is_none());
    let at = repos::Model::at(db, &format!("{}/", two.display())).await;
    assert_eq!(at.unwrap().unwrap().id, local.id);
}

#[tokio::test]
#[serial]
async fn the_clones_origin_says_the_project() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("repos-origin");
    let shell = sandbox.shell();
    let cloned = clone(
        &sandbox,
        "cloned",
        Some("git@git.example.com:group/project.git"),
    );
    let alone = clone(&sandbox, "alone", None);

    let repo = repos::Model::create(db, &shell, &params(&cloned, "")).await;
    let repo = repo.unwrap().unwrap();
    assert_eq!(repo.url, "https://git.example.com/group/project");
    // A clone with no origin has no project, and neither has a directory that's no clone.
    for dir in [alone, sandbox.repo()] {
        let repo = repos::Model::create(db, &shell, &params(&dir, "")).await;
        assert_eq!(repo.unwrap().unwrap().url, "");
    }

    // What you write wins: a fork's origin isn't the project that you follow.
    let upstream = Changes {
        url: Some("https://git.example.com/upstream/project".to_string()),
        ..Changes::default()
    };
    let changed = repos::Model::change(db, &shell, repo.id, upstream).await;
    let changed = changed.unwrap().unwrap();
    assert_eq!(
        (changed.path.as_str(), changed.url.as_str()),
        (
            repo.path.as_str(),
            "https://git.example.com/upstream/project"
        )
    );
    // And blank asks the clone again.
    let blank = Changes {
        url: Some(String::new()),
        ..Changes::default()
    };
    let read = repos::Model::change(db, &shell, repo.id, blank).await;
    assert_eq!(read.unwrap().unwrap().url, repo.url);
}

#[tokio::test]
#[serial]
async fn a_repo_that_is_in_use_stays() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let db = &ctx.db;
    let repo = support::repo(db, "/clones/r", "https://github.com/o/r").await;
    let item = support::watch(ctx, "https://github.com/o/r/issues/1").await;
    let schedule = support::unchecked(db, "triage", "/clones/r", "cat").await;
    let run = support::queue_one(db, &schedule).await;
    assert_eq!(repo.uses(db).await.unwrap(), (1, 1));

    let refused = repo.clone().removed(db).await.unwrap();
    assert_eq!(
        refused.unwrap_err(),
        "\"/clones/r\" is in use: by 1 watchlist items and 1 schedules. Take them off it first."
    );
    item.delete(db).await.unwrap();
    assert!(repo.clone().removed(db).await.unwrap().is_err());

    // With nothing in it, it goes. Its runs stay in the history, with the clone's path.
    schedule.delete(db).await.unwrap();
    repo.removed(db).await.unwrap().unwrap();
    assert!(repos::Model::list(db).await.unwrap().is_empty());
    let kept = runs::Model::find(db, run.id).await.unwrap();
    assert_eq!((kept.repo.as_str(), kept.repo_id), ("/clones/r", None));
}

#[tokio::test]
#[serial]
async fn a_tick_looks_at_the_repos() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("repos-check");
    let shell = sandbox.shell();
    let origin = "https://git.example.com/group/project.git";
    let cloned = clone(&sandbox, "cloned", Some(origin));
    let gone = sandbox.path("gone");
    // Rows from before m8r had repos: nobody checked them, and nobody said their projects.
    let known = support::repo(db, &cloned.to_string_lossy(), "").await;
    let lost = support::repo(db, &gone.to_string_lossy(), "").await;

    repos::Model::check(db, &shell).await.unwrap();

    // The clone's origin says its project. A clone that isn't there is an error.
    let read = repos::Model::find(db, known.id).await.unwrap();
    assert_eq!(read.url, "https://git.example.com/group/project");
    let wrong = open(db).await;
    assert_eq!(wrong.len(), 1);
    assert_eq!(wrong[0].0, format!("repo {}", lost.path));
    assert!(
        wrong[0].1.starts_with("The clone isn't there:"),
        "{}",
        wrong[0].1
    );

    // Its clone comes, of the project that another repo has: a project has one repo.
    clone(&sandbox, "gone", Some(origin));
    repos::Model::check(db, &shell).await.unwrap();
    let wrong = open(db).await;
    assert_eq!(wrong.len(), 1);
    assert_eq!(
        wrong[0].1,
        format!(
            "Its origin is https://git.example.com/group/project, which is the project of the \
             repo {}. A project has one repo, so remove one of the two.",
            known.path
        )
    );
    assert_eq!(repos::Model::find(db, lost.id).await.unwrap().url, "");

    // The other repo goes, and the next look puts it right.
    repos::Model::find(db, known.id)
        .await
        .unwrap()
        .removed(db)
        .await
        .unwrap()
        .unwrap();
    repos::Model::check(db, &shell).await.unwrap();
    assert!(open(db).await.is_empty());
    let read = repos::Model::find(db, lost.id).await.unwrap();
    assert_eq!(read.url, "https://git.example.com/group/project");
}

#[tokio::test]
#[serial]
async fn a_repo_that_moves_takes_no_error_along() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let sandbox = Sandbox::new("repos-move");
    let shell = sandbox.shell();
    let (old, new) = (sandbox.path("old"), sandbox.another_repo("new"));
    let repo = support::repo(db, &old.to_string_lossy(), "").await;
    repos::Model::check(db, &shell).await.unwrap();
    // A run failed there too, and an error is about another repo.
    let of_schedule = errors::schedule(3, &repo.path);
    errors::Model::raise(db, &of_schedule, "Run 1 (triage) failed.", None)
        .await
        .unwrap();
    errors::Model::raise(db, "repo /elsewhere", "The clone isn't there.", None)
        .await
        .unwrap();
    assert_eq!(open(db).await.len(), 3);

    // Its clone is somewhere else now. The errors named the path from before.
    let moved = Changes {
        path: Some(new.to_string_lossy().into_owned()),
        ..Changes::default()
    };
    let changed = repos::Model::change(db, &shell, repo.id, moved).await;
    assert_eq!(changed.unwrap().unwrap().dir(), new);
    let left: Vec<String> = open(db).await.into_iter().map(|error| error.0).collect();
    assert_eq!(left, ["repo /elsewhere"]);
}
