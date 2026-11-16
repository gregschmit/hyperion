//! The way from an older database to this one: an update mustn't lose what's there.

use std::fs;

use jiff::civil::date;
use loco_rs::prelude::*;
use m8r::{
    models::{categories, configurations, runs, schedules, tokens, watchlist_items},
    prompts,
};
use migration::{Migrator, MigratorTrait};
use sea_orm::Database;

/// A database of its own, with no migration run yet.
async fn empty(name: &str) -> DatabaseConnection {
    let dir = std::env::current_dir().unwrap().join("target/test-data");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{name}.sqlite"));
    let _ = fs::remove_file(&file);
    Database::connect(format!("sqlite://{}?mode=rwc", file.display()))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_database_of_the_first_release_keeps_working() {
    let db = empty("first-release").await;
    // The first release had the first migration alone: items, a prompt for each operation in
    // the configuration, and schedules with a cron expression and no command.
    Migrator::up(&db, Some(1)).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO configurations (name, active, system_message, triage_prompt, review_prompt)
         VALUES
           ('Old', FALSE, 'An old one.', 'Old triage.', 'Old review.'),
           ('First', TRUE,
            'These have priority, as the items say:
{% for item in items %}- {{ item.priority }}: {{ item.url }} {{ item.note }}
{% else %}(none)
{% endfor %}',
            '{{ system_message }} Triage the new items.',
            '{{ system_message }}
Review them:{%- for item in items -%} {{ item.url }}{%- endfor %}');
         INSERT INTO schedules (operation, repo, cron) VALUES
           ('triage', '/home/m8r/git/rxg', '30 6 * * 1-5'),
           ('review', '/home/m8r/git/rxg', '0 12 * * *'),
           ('develop', '/home/m8r/git/infrastructure', '0 1 * * 6');
         INSERT INTO items (url, priority, note, title) VALUES
           ('https://github.com/o/r/pull/1', 'high', 'Soon', 'A pull request'),
           ('https://github.com/o/r/issues/2', 'medium', '', ''),
           ('https://git.example.com/g/p/-/merge_requests/3', 'medium', 'Alice asked', '');
         INSERT INTO runs (operation, repo, status) VALUES
           ('triage', '/home/m8r/git/rxg', 'ok'),
           ('review', '/home/m8r/git/rxg', 'queued');",
    )
    .await
    .unwrap();

    Migrator::up(&db, None).await.unwrap();

    // The items are watchlist items, each in the category that its URL says. Nobody asked
    // for a review.
    let watchlist: Vec<_> = watchlist_items::Model::list(&db, None)
        .await
        .unwrap()
        .into_iter()
        .map(|(item, category)| {
            assert!(!item.review_requested);
            (
                item.url,
                category.name,
                item.priority,
                item.note,
                item.title,
            )
        })
        .collect();
    let item = |url: &str, category: &str, priority: &str, note: &str, title: &str| {
        (
            url.to_string(),
            category.to_string(),
            priority.to_string(),
            note.to_string(),
            title.to_string(),
        )
    };
    assert_eq!(
        watchlist,
        [
            item("https://github.com/o/r/issues/2", "Issue", "medium", "", ""),
            item(
                "https://github.com/o/r/pull/1",
                "Pull/Merge Request",
                "high",
                "Soon",
                "A pull request"
            ),
            item(
                "https://git.example.com/g/p/-/merge_requests/3",
                "Pull/Merge Request",
                "medium",
                "Alice asked",
                ""
            ),
        ]
    );
    assert_eq!(categories::Model::list(&db).await.unwrap().len(), 2);

    // Each schedule has the command that every run had (the agent's, from the settings), the
    // prompt that the active configuration had for its operation, and the category that fits
    // its operation.
    let agent = std::env::var("M8R_AGENT").unwrap_or_else(|_| "cat".to_string());
    let schedules: Vec<_> = schedules::Model::list(&db).await.unwrap();
    let by_operation = |operation: &str| {
        let found = schedules.iter().find(|it| it.0.operation == operation);
        found.unwrap().clone()
    };
    let (triage, issues) = by_operation("triage");
    let (review, prs) = by_operation("review");
    let (develop, developed) = by_operation("develop");
    assert_eq!(triage.command, agent);
    assert_eq!(triage.prompt, "{{ system_message }} Triage the new items.");
    assert_eq!(develop.prompt, "");
    let (issues, prs) = (issues.unwrap(), prs.unwrap());
    assert_eq!(
        (issues.name.as_str(), prs.name.as_str()),
        ("Issue", "Pull/Merge Request")
    );
    assert_eq!(developed.unwrap().id, issues.id);
    // Its cron expression still says when. 2026-11-16 is a Monday.
    assert_eq!(triage.hron, "30 6 * * 1-5");
    let monday = |hour, minute| {
        date(2026, 11, 16)
            .at(hour, minute, 0, 0)
            .in_tz("America/Chicago")
            .unwrap()
    };
    assert!(triage.is_due(&monday(6, 30)));
    assert!(!triage.is_due(&monday(6, 31)));

    // The templates loop over the list by its new name, and compile. What isn't a template's
    // tag stays as it was written.
    let active = configurations::Model::active(&db).await.unwrap().unwrap();
    assert!(active.system_message.starts_with(
        "These have priority, as the items say:\n{% for item in watchlist_items %}- "
    ));
    assert!(
        review.prompt.ends_with(
            "Review them:{%- for item in watchlist_items -%} {{ item.url }}{%- endfor %}"
        ),
        "{}",
        review.prompt
    );
    // A triage gets the issues, and a review the PRs/MRs.
    let context = prompts::Context::load(&db, Some(&issues), "/x")
        .await
        .unwrap();
    assert_eq!(
        context.compile(&triage.prompt).unwrap(),
        "These have priority, as the items say:\n\
         - medium: https://github.com/o/r/issues/2 Triage the new items."
    );
    let context = prompts::Context::load(&db, Some(&prs), "/x").await.unwrap();
    assert_eq!(
        context.compile(&review.prompt).unwrap(),
        "These have priority, as the items say:\n\
         - high: https://github.com/o/r/pull/1 Soon\n\
         - medium: https://git.example.com/g/p/-/merge_requests/3 Alice asked\n\
         Review them:https://github.com/o/r/pull/1\
         https://git.example.com/g/p/-/merge_requests/3"
    );

    // The history has the command too. The run that waited has its schedule, so it starts.
    let history = runs::Model::recent(&db, 10).await.unwrap();
    assert!(history.iter().all(|run| run.command == agent));
    let waiting = runs::Model::next(&db).await.unwrap().unwrap();
    assert_eq!(waiting.schedule_id, Some(review.id));
    let done = history.iter().find(|run| run.status == "ok").unwrap();
    assert_eq!(done.schedule_id, None);

    assert!(tokens::Model::list(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_new_database_starts_with_the_two_categories() {
    let db = empty("new").await;
    Migrator::up(&db, None).await.unwrap();

    let names: Vec<_> = categories::Model::list(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|category| category.name)
        .collect();
    assert_eq!(names, ["Issue", "Pull/Merge Request"]);

    // The last migration moves the prompts, and says that it can't be taken back.
    let back = Migrator::down(&db, Some(1)).await;
    assert!(back
        .unwrap_err()
        .to_string()
        .contains("can't be taken back"));
    assert_eq!(categories::Model::list(&db).await.unwrap().len(), 2);
}
