//! The way from an older database to this one: an update mustn't lose what it says it keeps.

use std::{
    fs,
    time::{Duration, SystemTime},
};

use loco_rs::prelude::*;
use m8r::{
    models::{
        errors, repos, runs, schedules, tokens,
        watchlist_items::{self, Filter},
    },
    prompts::Context,
};
use migration::{schema, Migrator, MigratorTrait};
use sea_orm::{
    sea_query::{Alias, Query},
    Database, DbBackend, Statement,
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

/// What a database of the fourth release holds: a repo, a schedule in it, an item, a run with
/// an artifact, the schedule's error, a configuration that isn't active, the error that says
/// so, and a token.
const FOURTH_RELEASE: &str = "
    INSERT INTO configurations (name, active, prompt) VALUES ('Mine', FALSE, 'Be brief.');
    INSERT INTO repos (path, url) VALUES ('~/git/rxg', 'https://git.example.com/rgnets/rxg');
    INSERT INTO schedules (name, category, max_items, hron, command, prompt) VALUES
      ('review', 'pull/merge requests', 5, 'every weekday at 06:00', 'claude -p',
       '{{ watchlist }}');
    INSERT INTO schedule_repos (schedule_id, repo_id) VALUES (1, 1);
    INSERT INTO watchlist_items (url, repo_id, category, requested) VALUES
      ('https://git.example.com/rgnets/rxg/-/merge_requests/1', 1, 'pull/merge requests', TRUE);
    INSERT INTO runs (name, repo, command, status, schedule_id, repo_id) VALUES
      ('review', '~/git/rxg', 'claude -p', 'failed', 1, 1);
    INSERT INTO artifacts (run_id, name, url) VALUES (1, 'review', '');
    INSERT INTO errors (subject, message, run_id) VALUES
      ('schedule 1 in ~/git/rxg', 'Run 1 (review) failed: it broke', 1),
      ('configuration', 'No configuration is active, so nothing runs.', NULL);
    INSERT INTO tokens (name, digest, scope) VALUES ('duet', 'a', 'watchlist_items');";

/// The ids in `table`, or an error if there's no such table.
async fn ids(db: &DatabaseConnection, table: &str) -> Result<usize, DbErr> {
    let all = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new(table))
        .to_owned();
    Ok(db.query_all(&all).await?.len())
}

#[tokio::test]
async fn an_update_keeps_everything_but_the_configurations_and_the_artifacts() {
    let db = empty("fourth-release").await;
    // The fourth release: m8r ran nothing until a configuration was active.
    Migrator::up(&db, Some(4)).await.unwrap();
    db.execute_unprepared(FOURTH_RELEASE).await.unwrap();

    Migrator::up(&db, None).await.unwrap();

    // The schedule with its repo, the watchlist, the history and the token are there.
    let scheduled = schedules::Model::list(&db).await.unwrap();
    assert_eq!(scheduled.len(), 1);
    let (review, in_repos) = &scheduled[0];
    assert_eq!(
        (
            review.name.as_str(),
            review.max_items,
            in_repos[0].path.as_str()
        ),
        ("review", 5, "~/git/rxg")
    );
    // Its times are in the column that says what they are.
    assert_eq!(review.when, "every weekday at 06:00");
    // A schedule that was there is enabled: it goes on as it did.
    assert!(review.enabled);
    let items = watchlist_items::Model::list(&db, Filter::default()).await;
    assert_eq!(items.unwrap().len(), 1);
    let history = runs::Model::recent(&db, 10).await.unwrap();
    assert_eq!(
        (history[0].schedule_id, history[0].repo_id),
        (Some(1), Some(1))
    );
    assert_eq!(tokens::Model::list(&db).await.unwrap().len(), 1);

    // The artifacts are gone: a run's report is what it printed.
    assert!(ids(&db, "artifacts").await.is_err());
    // The configurations are gone, and the error that asked for one is over: the list keeps
    // it. The schedule's error stays.
    assert!(ids(&db, "configurations").await.is_err());
    let open = errors::Model::open(&db).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].subject, "schedule 1 in ~/git/rxg");
    let kept = errors::Model::list(&db, 10).await.unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[1].subject, "configuration");
    assert!(kept[1].cleared_at.is_some());

    // And the schedule runs with what's there.
    let queued = runs::Model::queue(&db, review, in_repos).await.unwrap();
    assert_eq!((queued[0].id, queued[0].status.as_str()), (2, "queued"));

    // The artifacts don't come back, so there's no way back.
    let back = Migrator::down(&db, Some(1)).await;
    assert!(back
        .unwrap_err()
        .to_string()
        .contains("can't be taken back"));
}

#[tokio::test]
async fn a_schedules_prompt_gets_what_the_active_configuration_held() {
    // What the configuration held, whether it was the active one, and what a run gets after
    // the update: what it got before it.
    let marks = "Use {{ braces }}, {% tags %} and {# notes #}.";
    for (name, held, active, gets) in [
        // The box's own case: m8r wanted a configuration, and this one said nothing.
        ("blank", "", true, ""),
        ("text", "Be brief.\n", true, "Be brief."),
        // It was plain text, whatever marks it had.
        ("marks", marks, true, marks),
        ("not-active", "Be brief.", false, ""),
    ] {
        let db = empty(&format!("held-{name}")).await;
        Migrator::up(&db, Some(4)).await.unwrap();
        let insert = |sql: &'static str, values: Vec<sea_orm::Value>| {
            db.execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                sql,
                values,
            ))
        };
        insert(
            "INSERT INTO configurations (name, active, prompt) VALUES ('Mine', ?, ?)",
            vec![active.into(), held.into()],
        )
        .await
        .unwrap();
        // One schedule names the configuration's prompt, as a form sends it and without the
        // spaces. One doesn't name it.
        for (schedule, prompt) in [
            (
                "names it",
                "Before.\r\n{{ config_prompt }}\r\nAfter. [{{config_prompt}}]",
            ),
            ("other", "{{ watchlist }}"),
        ] {
            insert(
                "INSERT INTO schedules (name, hron, command, prompt)
                 VALUES (?, 'every day at 9:00', 'cat', ?)",
                vec![schedule.into(), prompt.into()],
            )
            .await
            .unwrap();
        }

        Migrator::up(&db, None).await.unwrap();

        let scheduled = schedules::Model::list(&db).await.unwrap();
        let compiled = Context::default().compile(&scheduled[0].0.prompt);
        assert_eq!(
            compiled.unwrap(),
            format!("Before.\r\n{gets}\r\nAfter. [{gets}]"),
            "{name}: {}",
            scheduled[0].0.prompt
        );
        assert_eq!(scheduled[1].0.prompt, "{{ watchlist }}", "{name}");
    }
}

#[tokio::test]
async fn a_migration_runs_on_connections_that_know_the_schema_from_before() {
    // The app's pool has several connections, and each keeps the schema that it read last.
    // SQLite takes a quoted name that it doesn't know for text, so a query after a migration
    // must not go wrong on a connection that hasn't seen the change.
    let dir = std::env::current_dir().unwrap().join("target/test-data");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("several-connections.sqlite");
    let _ = fs::remove_file(&file);
    let mut options = sea_orm::ConnectOptions::new(format!("sqlite://{}?mode=rwc", file.display()));
    options.min_connections(3).max_connections(3);
    let db = Database::connect(options).await.unwrap();
    Migrator::up(&db, Some(4)).await.unwrap();
    db.execute_unprepared(FOURTH_RELEASE).await.unwrap();
    // Every connection reads the schema as it is now.
    let look = || async {
        for table in ["configurations", "artifacts", "schedules", "runs", "errors"] {
            ids(&db, table).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    for _ in 0..3 {
        tokio::join!(look(), look(), look());
    }

    Migrator::up(&db, None).await.unwrap();

    // Whichever connection answers, it reads what the migration made.
    let read = || async {
        assert!(ids(&db, "configurations").await.is_err());
        assert!(ids(&db, "artifacts").await.is_err());
        assert_eq!(schedules::Model::list(&db).await.unwrap().len(), 1);
        let history = runs::Model::recent(&db, 10).await.unwrap();
        assert_eq!(history[0].name, "review");
        assert_eq!(errors::Model::open(&db).await.unwrap().len(), 1);
    };
    for _ in 0..3 {
        tokio::join!(read(), read(), read());
    }
}

#[tokio::test]
async fn a_migration_that_fails_changes_nothing() {
    let db = empty("fails").await;
    Migrator::up(&db, Some(4)).await.unwrap();
    db.execute_unprepared(FOURTH_RELEASE).await.unwrap();
    // Something is in the way of a late step: no error can be cleared.
    db.execute_unprepared(
        "CREATE TRIGGER in_the_way BEFORE UPDATE ON errors
         BEGIN SELECT RAISE(ABORT, 'in the way'); END",
    )
    .await
    .unwrap();

    assert!(Migrator::up(&db, None).await.is_err());

    // The database is still the fourth release's, whole: the m8r that runs goes on with it.
    assert_eq!(ids(&db, "configurations").await.unwrap(), 1);
    assert_eq!(errors::Model::open(&db).await.unwrap().len(), 2);

    // With that out of the way, the same migration goes through.
    db.execute_unprepared("DROP TRIGGER in_the_way")
        .await
        .unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert!(ids(&db, "configurations").await.is_err());
    assert_eq!(errors::Model::open(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_new_database_starts_with_no_repos() {
    let db = empty("new").await;
    Migrator::up(&db, None).await.unwrap();

    assert!(repos::Model::list(&db).await.unwrap().is_empty());
    assert!(schedules::Model::list(&db).await.unwrap().is_empty());
    // A migration that can't be taken back says so.
    let back = Migrator::down(&db, Some(1)).await;
    assert!(back
        .unwrap_err()
        .to_string()
        .contains("can't be taken back"));
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
