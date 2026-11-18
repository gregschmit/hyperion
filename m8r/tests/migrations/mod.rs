//! The way from an older database to this one: an update mustn't lose what it says it keeps.

use std::{
    fs,
    time::{Duration, SystemTime},
};

use loco_rs::prelude::*;
use m8r::models::{
    configurations, errors, repos,
    runs::{self, Status},
    schedules, tokens,
    watchlist_items::{self, Filter},
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

/// What a database of the third release holds: a schedule in a category, items, runs, an
/// error of the schedule and one of a tool, and a token.
const THIRD_RELEASE: &str = "
    INSERT INTO configurations (name, active, prompt) VALUES ('Mine', TRUE, 'Be brief.');
    INSERT INTO schedules (operation, repos, hron, command, prompt, category_id) VALUES
      ('review', '~/git/rxg', 'every weekday at 06:00', 'claude -p', '{{ watchlist }}', 2);
    INSERT INTO watchlist_items (url, category_id, review_requested) VALUES
      ('https://git.example.com/rgnets/rxg/-/merge_requests/1', 2, TRUE),
      ('https://git.example.com/rgnets/rxg/-/issues/2', 1, FALSE);
    INSERT INTO runs (operation, repo, command, status, schedule_id) VALUES
      ('review', '~/git/rxg', 'claude -p', 'failed', 1),
      ('review', '~/git/rxg', 'claude -p', 'queued', 1);
    INSERT INTO errors (subject, message, run_id) VALUES
      ('schedule 1 in ~/git/rxg', 'Run 1 (review) failed: it broke', 1),
      ('gh', 'm8r finds no such command.', NULL);
    INSERT INTO tokens (name, digest, scope) VALUES ('duet', 'a', 'watchlist_items');";

#[tokio::test]
async fn an_update_keeps_the_history_and_starts_the_schedules_and_the_watchlist_new() {
    let db = empty("third-release").await;
    // The third release: a schedule had an operation, a category and its clones' paths, and
    // an item had a category. Neither named a repo.
    Migrator::up(&db, Some(3)).await.unwrap();
    db.execute_unprepared(THIRD_RELEASE).await.unwrap();

    Migrator::up(&db, None).await.unwrap();

    // m8r has no repo yet, so no schedule and no item: those are yours to add again.
    assert!(repos::Model::list(&db).await.unwrap().is_empty());
    assert!(schedules::Model::list(&db).await.unwrap().is_empty());
    let all = watchlist_items::Model::list(&db, Filter::default()).await;
    assert!(all.unwrap().is_empty());

    // The rest is there. The history has each run, by the name that its schedule had, with no
    // schedule and no repo.
    let active = configurations::Model::active(&db).await.unwrap().unwrap();
    assert_eq!(active.prompt, "Be brief.");
    assert_eq!(tokens::Model::list(&db).await.unwrap().len(), 1);
    let history = runs::Model::recent(&db, 10).await.unwrap();
    let runs: Vec<_> = history
        .iter()
        .map(|run| {
            assert_eq!((run.schedule_id, run.repo_id), (None, None));
            (run.name.as_str(), run.repo.as_str(), run.status.as_str())
        })
        .collect();
    assert_eq!(
        runs,
        [
            ("review", "~/git/rxg", "queued"),
            ("review", "~/git/rxg", "failed")
        ]
    );
    // The error of the schedule that's gone is over, and the list keeps it. The tool's stays.
    let open = errors::Model::open(&db).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].subject, "gh");
    let kept = errors::Model::list(&db, 10).await.unwrap();
    assert_eq!(kept.len(), 2);
    assert!(kept[1].cleared_at.is_some());

    // And the new tables work with the old ones: a repo, an item in it, a schedule there, and
    // a run of it beside the history.
    let mut repo = repos::ActiveModel::new();
    repo.path = Set("~/git/rxg".to_string());
    repo.url = Set("https://git.example.com/rgnets/rxg".to_string());
    let repo = repo.insert(&db).await.unwrap();
    let item = watchlist_items::Params {
        url: "https://git.example.com/rgnets/rxg/-/merge_requests/1".to_string(),
        ..watchlist_items::Params::default()
    };
    let (item, _) = watchlist_items::Model::create(&db, &item)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((item.id, item.repo_id), (1, repo.id));
    let mut review = schedules::ActiveModel::new();
    review.name = Set("review".to_string());
    review.hron = Set("every weekday at 06:00".to_string());
    let review = review.insert(&db).await.unwrap();
    let queued = runs::Model::queue(&db, &review, std::slice::from_ref(&repo)).await;
    let run = queued.unwrap().remove(0).into_active_model();
    let run = run
        .finished(&db, Status::Ok, String::new(), String::new())
        .await
        .unwrap();
    assert_eq!((run.id, run.repo_id), (3, Some(repo.id)));

    // The schedules and the watchlist of before are gone, so there's no way back.
    let back = Migrator::down(&db, Some(1)).await;
    assert!(back
        .unwrap_err()
        .to_string()
        .contains("can't be taken back"));
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
    Migrator::up(&db, Some(3)).await.unwrap();
    db.execute_unprepared(THIRD_RELEASE).await.unwrap();
    // Every connection reads the schema as it is now.
    let look = || async {
        for table in ["schedules", "watchlist_items", "runs", "errors"] {
            let seen = Query::select()
                .column(Alias::new("id"))
                .from(Alias::new(table))
                .to_owned();
            db.query_all(&seen).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    for _ in 0..3 {
        tokio::join!(look(), look(), look());
    }

    Migrator::up(&db, None).await.unwrap();

    // Whichever connection answers, it reads what the migration made.
    let read = || async {
        assert!(schedules::Model::list(&db).await.unwrap().is_empty());
        let items = watchlist_items::Model::list(&db, Filter::default()).await;
        assert!(items.unwrap().is_empty());
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
    Migrator::up(&db, Some(3)).await.unwrap();
    // Something is in the way of a late step: a table of that name, with other columns.
    db.execute_unprepared(THIRD_RELEASE).await.unwrap();
    db.execute_unprepared("CREATE TABLE schedule_repos (id integer PRIMARY KEY)")
        .await
        .unwrap();

    assert!(Migrator::up(&db, None).await.is_err());

    // The database is still the third release's, whole: the m8r that runs goes on with it.
    let old = |table: &'static str, column: &'static str| {
        Query::select()
            .column(Alias::new(column))
            .from(Alias::new(table))
            .to_owned()
    };
    for (table, column, rows) in [
        ("schedules", "operation", 1),
        ("watchlist_items", "review_requested", 2),
        ("categories", "name", 2),
        ("runs", "operation", 2),
    ] {
        let kept = db.query_all(&old(table, column)).await;
        assert_eq!(kept.unwrap().len(), rows, "{table}.{column}");
    }
    assert!(db.query_all(&old("repos", "path")).await.is_err());
    assert_eq!(errors::Model::open(&db).await.unwrap().len(), 2);

    // With that out of the way, the same migration goes through.
    db.execute_unprepared("DROP TABLE schedule_repos")
        .await
        .unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert!(repos::Model::list(&db).await.unwrap().is_empty());
    assert_eq!(runs::Model::recent(&db, 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_new_database_starts_with_no_repos() {
    let db = empty("new").await;
    Migrator::up(&db, None).await.unwrap();

    assert!(repos::Model::list(&db).await.unwrap().is_empty());
    assert!(schedules::Model::list(&db).await.unwrap().is_empty());
    // The latest migration says that it can't be taken back.
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
