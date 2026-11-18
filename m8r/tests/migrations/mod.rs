//! The way from an older database to this one: an update mustn't lose what's there.

use std::{
    fs,
    time::{Duration, SystemTime},
};

use jiff::civil::date;
use loco_rs::prelude::*;
use m8r::{
    models::{categories, configurations, errors, runs, schedules, tokens, watchlist_items},
    prompts,
};
use migration::{schema, Migrator, MigratorTrait};
use sea_orm::{
    sea_query::{Alias, Query},
    Database,
};

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
    assert_eq!(triage.prompt, "{{ config_prompt }} Triage the new items.");
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
    // tag stays as it was written. What was the system message is the configuration's prompt.
    let active = configurations::Model::active(&db).await.unwrap().unwrap();
    assert!(active.prompt.starts_with(
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
async fn a_database_of_the_second_release_keeps_working() {
    let db = empty("second-release").await;
    // The second release: a configuration held the system message, a prompt could name it
    // and the skill, and what was wrong with a tool was in a table of its own.
    Migrator::up(&db, Some(2)).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO configurations (name, active, system_message) VALUES
           ('Mine', TRUE, 'The words skill and system_message stay.
{{ skill }}');
         INSERT INTO schedules (operation, repo, hron, command, prompt, category_id) VALUES
           ('review', '~/git/rxg', 'every day at 09:00', 'claude -p',
            '{{ system_message }}

{{skill}}

The watchlist now:
{{ watchlist }}
{% if system_message %}Review.{% endif %}', 2),
           ('other', '~/git/rxg', 'every 12 hours from 00:00 to 23:59', 'make docs', '', NULL);
         INSERT INTO watchlist_items (url, category_id) VALUES ('https://github.com/o/r/pull/1', 2);
         INSERT INTO runs (operation, repo, command, status, schedule_id)
           VALUES ('review', '~/git/rxg', 'claude -p', 'ok', 1);
         INSERT INTO tool_errors (tool, message) VALUES ('claude', 'It isn''t installed.');",
    )
    .await
    .unwrap();

    Migrator::up(&db, None).await.unwrap();

    // The parts of a prompt have their new names, in the tags and nowhere else.
    let active = configurations::Model::active(&db).await.unwrap().unwrap();
    assert_eq!(
        active.prompt,
        "The words skill and system_message stay.\n{{ m8r_prompt }}"
    );
    let schedules = schedules::Model::list(&db).await.unwrap();
    let (review, prs) = schedules
        .iter()
        .find(|it| it.0.operation == "review")
        .unwrap()
        .clone();
    assert_eq!(
        review.prompt,
        "{{ config_prompt }}\n\n{{m8r_prompt}}\n\nThe watchlist now:\n{{ watchlist }}\n\
         {% if config_prompt %}Review.{% endif %}"
    );
    // Its repo is the first of its repos, and the only one.
    assert_eq!(review.repos().collect::<Vec<_>>(), ["~/git/rxg"]);
    // And the prompt compiles to what it did: the configuration's part, m8r's, the items.
    let context = prompts::Context::load(&db, prs.as_ref(), "/x")
        .await
        .unwrap();
    let compiled = context.compile(&review.prompt).unwrap();
    assert!(
        compiled.starts_with("The words skill and system_message stay.\nYou are running in m8r"),
        "{compiled}"
    );
    assert!(
        compiled.ends_with(
            "The watchlist now:\n\
             - item 1 (Pull/Merge Request, medium): https://github.com/o/r/pull/1\nReview."
        ),
        "{compiled}"
    );

    // The history is there, and the list of errors starts empty.
    assert_eq!(runs::Model::recent(&db, 10).await.unwrap().len(), 1);
    assert!(errors::Model::list(&db, 10).await.unwrap().is_empty());

    // This migration can be taken back, and run again.
    Migrator::down(&db, Some(1)).await.unwrap();
    let back = db
        .query_one(
            Query::select()
                .column(Alias::new("system_message"))
                .from(Alias::new("configurations")),
        )
        .await
        .unwrap()
        .unwrap();
    let text: String = back.try_get("", "system_message").unwrap();
    assert_eq!(
        text,
        "The words skill and system_message stay.\n{{ skill }}"
    );
    Migrator::up(&db, None).await.unwrap();
    let active = configurations::Model::active(&db).await.unwrap().unwrap();
    assert!(active.prompt.ends_with("{{ m8r_prompt }}"));
}

#[tokio::test]
async fn a_migration_runs_on_connections_that_know_the_schema_from_before() {
    // The app's pool has several connections, and each keeps the schema that it read last.
    // SQLite takes a quoted name that it doesn't know for text, so a query for a column that
    // another connection just made must not go wrong on a connection that hasn't seen it.
    let dir = std::env::current_dir().unwrap().join("target/test-data");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("several-connections.sqlite");
    let _ = fs::remove_file(&file);
    let mut options = sea_orm::ConnectOptions::new(format!("sqlite://{}?mode=rwc", file.display()));
    options.min_connections(3).max_connections(3);
    let db = Database::connect(options).await.unwrap();
    Migrator::up(&db, Some(2)).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO configurations (name, active, system_message) VALUES ('Mine', TRUE, '{{ skill }}');
         INSERT INTO schedules (operation, repo, hron, command, prompt)
           VALUES ('review', '~/git/rxg', 'every day at 09:00', 'claude -p', '{{ system_message }}');",
    )
    .await
    .unwrap();
    // Every connection reads the schema as it is now.
    let look = || async {
        let seen = Query::select()
            .column(Alias::new("name"))
            .from(Alias::new("configurations"))
            .to_owned();
        db.query_all(&seen).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    for _ in 0..3 {
        tokio::join!(look(), look(), look());
    }

    Migrator::up(&db, None).await.unwrap();

    let active = configurations::Model::active(&db).await.unwrap().unwrap();
    assert_eq!(active.prompt, "{{ m8r_prompt }}");
    let schedules = schedules::Model::list(&db).await.unwrap();
    assert_eq!(schedules[0].0.prompt, "{{ config_prompt }}");
}

#[tokio::test]
async fn a_migration_that_fails_changes_nothing() {
    let db = empty("fails").await;
    Migrator::up(&db, Some(2)).await.unwrap();
    // Something is in the way of the last step: a table of that name, with other columns.
    db.execute_unprepared(
        "INSERT INTO configurations (name, active, system_message) VALUES ('Mine', TRUE, '{{ skill }}');
         CREATE TABLE errors (id integer PRIMARY KEY);",
    )
    .await
    .unwrap();

    assert!(Migrator::up(&db, None).await.is_err());

    // The database is still the second release's, whole: the m8r that runs goes on with it.
    let system_message = Query::select()
        .column(Alias::new("system_message"))
        .from(Alias::new("configurations"))
        .to_owned();
    let kept = db.query_one(&system_message).await.unwrap().unwrap();
    let text: String = kept.try_get("", "system_message").unwrap();
    assert_eq!(text, "{{ skill }}");
    let tool_errors = Query::select()
        .column(Alias::new("tool"))
        .from(Alias::new("tool_errors"))
        .to_owned();
    assert!(db.query_all(&tool_errors).await.is_ok());

    // With that out of the way, the same migration goes through.
    db.execute_unprepared("DROP TABLE errors").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let active = configurations::Model::active(&db).await.unwrap().unwrap();
    assert_eq!(active.prompt, "{{ m8r_prompt }}");
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

    // The migration that moved the prompts says that it can't be taken back.
    let back = Migrator::down(&db, Some(2)).await;
    assert!(back
        .unwrap_err()
        .to_string()
        .contains("can't be taken back"));
    assert_eq!(categories::Model::list(&db).await.unwrap().len(), 2);
}

#[tokio::test]
async fn the_schema_file_is_what_the_migrations_make() {
    let made = schema::sql().await.unwrap();
    let file = fs::read_to_string(schema::FILE).unwrap();
    assert!(
        file == made,
        "{} is behind the migrations. `cargo loco db migrate` writes it. They make:\n{made}",
        schema::FILE
    );
    // m8r's tables, and not what SQLite and the migrator keep for themselves.
    assert!(made.contains("\nCREATE TABLE \"watchlist_items\" (\n  \"created_at\" "));
    assert!(!made.contains("sqlite_sequence"));
    assert!(!made.contains("seaql_migrations"));
}

#[tokio::test]
async fn the_schema_is_laid_out_to_read() {
    let db = empty("laid-out").await;
    // SQL as a migration may write it by hand: lines broken anywhere, and commas in quotes and
    // in parentheses, which split nothing.
    db.execute_unprepared(
        "CREATE TABLE b (id integer PRIMARY KEY AUTOINCREMENT,
           kind varchar   NOT NULL DEFAULT 'a, (b)', amount decimal(10, 2),
           a_id integer , CHECK (kind IN ('a, (b)', 'it''s')),
           FOREIGN KEY (a_id, kind) REFERENCES a (id, kind)
             ON DELETE CASCADE);
         CREATE TABLE a (id integer, kind varchar, PRIMARY KEY (id, kind)) WITHOUT ROWID;
         CREATE INDEX b_kind ON b (kind, amount);
         CREATE UNIQUE INDEX b_amount ON b (amount) WHERE amount > 0;
         CREATE TABLE c (id integer, -- A comment ends with its line.\n  name varchar);",
    )
    .await
    .unwrap();

    // The tables by name, each with its indexes. The one with a comment is as it was written.
    assert_eq!(
        schema::of(&db).await.unwrap(),
        r"
CREATE TABLE a (
  id integer,
  kind varchar,
  PRIMARY KEY (id, kind)
) WITHOUT ROWID;

CREATE TABLE b (
  id integer PRIMARY KEY AUTOINCREMENT,
  kind varchar NOT NULL DEFAULT 'a, (b)',
  amount decimal(10, 2),
  a_id integer,
  CHECK (kind IN ('a, (b)', 'it''s')),
  FOREIGN KEY (a_id, kind) REFERENCES a (id, kind) ON DELETE CASCADE
);
CREATE UNIQUE INDEX b_amount ON b (amount) WHERE amount > 0;
CREATE INDEX b_kind ON b (kind, amount);

CREATE TABLE c (id integer, -- A comment ends with its line.
  name varchar);
"
    );
}

#[tokio::test]
async fn the_schema_file_is_written_when_it_says_something_else() {
    let dir = std::env::current_dir().unwrap().join("target/test-data");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("schema.sql");
    let _ = fs::remove_file(&file);

    schema::write(&file).await.unwrap();
    let made = schema::sql().await.unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), made);

    // It's left alone while it says the same, so nothing that watches the source restarts.
    let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let held = fs::File::options().write(true).open(&file).unwrap();
    held.set_modified(long_ago).unwrap();
    drop(held);
    schema::write(&file).await.unwrap();
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), long_ago);

    fs::write(&file, "-- An older schema.\n").unwrap();
    schema::write(&file).await.unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), made);

    // And an error says which file it is.
    let nowhere = dir.join("no-such-directory/schema.sql");
    let err = schema::write(&nowhere).await.unwrap_err().to_string();
    assert!(
        err.contains("no-such-directory/schema.sql can't be written"),
        "{err}"
    );
}

#[tokio::test]
async fn development_alone_keeps_the_schema_file() {
    // m8r itself, as `cargo loco db migrate` runs it, in a directory of its own.
    let root = std::env::current_dir().unwrap();
    let dir = root.join("target/test-data/schema-kept");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("migration")).unwrap();
    let file = dir.join(schema::FILE);
    let migrate = |environment: &'static str| {
        let database = format!("sqlite://{}/{environment}.sqlite?mode=rwc", dir.display());
        tokio::process::Command::new(env!("CARGO_BIN_EXE_m8r-cli"))
            .args(["db", "migrate"])
            .current_dir(&dir)
            .env("LOCO_ENV", environment)
            .env("LOCO_CONFIG_FOLDER", root.join("config"))
            .env("DATABASE_URL", database)
            // What production's settings have no default for.
            .env("HOST", "http://localhost:5150")
            .env("M8R_SECRET", "not-a-secret-".repeat(8))
            .env("M8R_DATA", &dir)
            .output()
    };

    // The box runs in its checkout, and mustn't write there. A test leaves the file to you.
    for environment in ["production", "test"] {
        let ran = migrate(environment).await.unwrap();
        let said = String::from_utf8_lossy(&ran.stderr);
        assert!(ran.status.success(), "{environment}: {said}");
        assert!(
            dir.join(format!("{environment}.sqlite")).exists(),
            "{environment} migrated nothing: {said}"
        );
        assert!(!file.exists(), "{environment} wrote the schema");
    }

    let ran = migrate("development").await.unwrap();
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(ran.status.success(), "{said}");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        schema::sql().await.unwrap()
    );
}
