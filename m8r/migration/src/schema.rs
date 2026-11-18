//! `schema.sql`, beside this crate's manifest: what the migrations make, in one place to read.
//!
//! Where m8r keeps the file (`keep`), `Migrator::up` writes it each time it has applied what
//! was pending. Development keeps it, and nothing else does.
//!
//! The schema is always SQLite's. It comes from a new database in memory, and not from the
//! database that m8r uses. So the file says the same whatever that database is, and whatever
//! was done to it by hand.

use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use sea_orm_migration::{
    prelude::*,
    sea_orm::{ConnectOptions, Database, DbBackend, Statement},
};

use crate::Migrator;

/// Where the file is, from the app's root: `cargo loco` runs there.
pub const FILE: &str = "migration/schema.sql";

const HEADER: &str = "\
-- What m8r's migrations make in a new SQLite database. In development, m8r writes this file
-- each time it runs its migrations: `cargo loco db migrate` does, and so does a start. Don't
-- edit it: it follows the migrations.
";

/// Whether this process keeps the file. `Migrator::up` gets a connection and nothing more, so
/// what the app knows about where it runs comes here.
static KEPT: AtomicBool = AtomicBool::new(false);

/// From now on, this process keeps the file.
pub fn keep() {
    KEPT.store(true, Ordering::Relaxed);
}

#[must_use]
pub fn is_kept() -> bool {
    KEPT.load(Ordering::Relaxed)
}

/// Write the schema to `file`, unless that's what it holds. A file that stays as it is
/// restarts nothing that watches the source.
///
/// # Errors
/// When a migration fails on a new database, or the file can't be written.
pub async fn write(file: &Path) -> Result<(), DbErr> {
    let sql = sql().await?;
    if fs::read_to_string(file).is_ok_and(|held| held == sql) {
        return Ok(());
    }
    fs::write(file, sql)
        .map_err(|err| DbErr::Migration(format!("{} can't be written: {err}", file.display())))
}

/// What the migrations make in a new database, as SQL.
///
/// # Errors
/// When a migration fails there.
pub async fn sql() -> Result<String, DbErr> {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.sqlx_logging(false);
    let db = Database::connect(options).await?;
    // Each migration's `up`, and not the migrator: that one logs every step, and keeps a table
    // of its own.
    let manager = SchemaManager::new(&db);
    for migration in Migrator::migrations() {
        migration.up(&manager).await.map_err(|err| {
            let name = migration.name();
            DbErr::Migration(format!("{name} fails on a new SQLite database: {err}"))
        })?;
    }
    Ok(format!("{HEADER}{}", of(&db).await?))
}

/// The schema of `db`, a SQLite database: its tables by name, each with its indexes.
///
/// # Errors
/// When the query fails.
pub async fn of(db: &impl ConnectionTrait) -> Result<String, DbErr> {
    let statements = Statement::from_string(
        DbBackend::Sqlite,
        // Not SQLite's own tables, such as `sqlite_sequence`.
        r"SELECT type, tbl_name, sql FROM sqlite_schema
          WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\_%' ESCAPE '\'
          ORDER BY tbl_name, type <> 'table', name",
    );
    let mut schema = String::new();
    let mut last = String::new();
    for row in db.query_all_raw(statements).await? {
        let kind: String = row.try_get("", "type")?;
        let table: String = row.try_get("", "tbl_name")?;
        let sql: String = row.try_get("", "sql")?;
        // A blank line before each table. Its indexes come right after it.
        if table != last {
            schema.push('\n');
        }
        schema.push_str(&laid_out(&kind, &sql));
        schema.push_str(";\n");
        last = table;
    }
    Ok(schema)
}

/// A statement as SQLite keeps it, laid out to read: a table's columns and constraints get a
/// line each. What isn't a table stays as it is.
fn laid_out(kind: &str, sql: &str) -> String {
    // A comment runs to the end of its line, so a statement that has one stays as it is too.
    if kind != "table" || sql.contains("--") || sql.contains("/*") {
        return sql.to_string();
    }
    let Some((head, rest)) = sql.split_once('(') else {
        return sql.to_string();
    };
    let Some((definition, tail)) = rest.rsplit_once(')') else {
        return sql.to_string();
    };
    let lines: Vec<String> = parts(definition)
        .iter()
        .map(|part| format!("  {part}"))
        .collect();
    format!("{} (\n{}\n){tail}", head.trim_end(), lines.join(",\n"))
}

/// Split a table's definition at its commas, into columns and constraints. A comma in quotes
/// or in parentheses is part of one. Each part comes out on one line: a migration that writes
/// its own SQL may break a line where it likes.
fn parts(definition: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    // The quote that we're inside, and how deep in parentheses we are.
    let mut quote = None;
    let mut depth = 0_u32;
    for c in definition.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                part.push(c);
            }
            None if c == ',' && depth == 0 => parts.push(std::mem::take(&mut part)),
            None if c.is_whitespace() => {
                if !part.ends_with(' ') {
                    part.push(' ');
                }
            }
            None => {
                match c {
                    '\'' | '"' | '`' => quote = Some(c),
                    '(' => depth += 1,
                    ')' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                part.push(c);
            }
        }
    }
    parts.push(part);
    parts.iter().map(|part| part.trim().to_string()).collect()
}
