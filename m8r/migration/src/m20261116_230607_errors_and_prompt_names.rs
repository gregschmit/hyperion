use loco_rs::schema::{create_table, drop_table, rename_column, ColType};
use sea_orm_migration::prelude::*;

/// What a template calls two parts of a prompt: the name until now, and the name from now on.
const NAMES: [(&str, &str); 2] = [("system_message", "config_prompt"), ("skill", "m8r_prompt")];

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// All of it happens, or none of it: SQLite takes a change to the schema back too. So a
    /// step that fails leaves the database as the m8r that's running knows it.
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // The templates first, while the schema is the one that every connection knows: the
        // parts of a prompt get their new names.
        for (old, new) in NAMES {
            rename_in_templates(m, "system_message", old, new).await?;
        }
        // A configuration's system message is its prompt: your part of every prompt. m8r's own
        // part is in the code.
        rename_column(m, "configurations", "system_message", "prompt").await?;
        // A schedule has one or more repos, one to a line, and a run starts in each. So does
        // its `hron`: one or more times, one to a line.
        rename_column(m, "schedules", "repo", "repos").await?;

        // What goes wrong is in one list, which keeps what's over. m8r looks for no tools.
        drop_table(m, "tool_errors").await?;
        // `run_id` references runs, ON DELETE SET NULL: the run that failed, if one did.
        create_table(
            m,
            "errors",
            &[
                ("id", ColType::PkAuto),
                // What it's about, such as `schedule 3`. What clears it names the same subject.
                ("subject", ColType::String),
                ("message", ColType::TextWithDefault(String::new())),
                // When it was cleared. Until then, every page shows it.
                ("cleared_at", ColType::TimestampWithTimeZoneNull),
            ],
            &[("run?", "")],
        )
        .await?;
        // A subject has one open error at most. A partial index, which sea-query can't build.
        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX idx_errors_one_open ON errors (subject) \
                 WHERE cleared_at IS NULL",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for (old, new) in NAMES {
            rename_in_templates(m, "prompt", new, old).await?;
        }
        rename_column(m, "configurations", "prompt", "system_message").await?;
        rename_column(m, "schedules", "repos", "repo").await?;

        drop_table(m, "errors").await?;
        create_table(
            m,
            "tool_errors",
            &[
                ("id", ColType::PkAuto),
                ("tool", ColType::String),
                ("host", ColType::StringWithDefault(String::new())),
                ("message", ColType::TextWithDefault(String::new())),
            ],
            &[],
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_tool_errors_tool_host")
                .table(Alias::new("tool_errors"))
                .col(Alias::new("tool"))
                .col(Alias::new("host"))
                .unique()
                .to_owned(),
        )
        .await
    }
}

/// Give the variable `from` the name `to` in every saved template: the schedules' prompts,
/// and the configurations' column `of_configurations`.
async fn rename_in_templates(
    m: &SchemaManager<'_>,
    of_configurations: &str,
    from: &str,
    to: &str,
) -> Result<(), DbErr> {
    let id = Alias::new("id");
    for (table, column) in [
        ("configurations", of_configurations),
        ("schedules", "prompt"),
    ] {
        let (table, column) = (Alias::new(table), Alias::new(column));
        // The columns have their table's name before them. A connection of the pool may know
        // an older schema, and SQLite takes a bare quoted name that it doesn't know for text.
        // With the table's name, it looks at the schema again.
        let rows = m
            .get_connection()
            .query_all(
                Query::select()
                    .columns([(table.clone(), id.clone()), (table.clone(), column.clone())])
                    .from(table.clone()),
            )
            .await?;
        for row in rows {
            let row_id: i64 = row.try_get("", "id")?;
            let template: String = row.try_get("", &column.to_string())?;
            let renamed = rename_in_tags(&template, from, to);
            if renamed == template {
                continue;
            }
            m.exec_stmt(
                Query::update()
                    .table(table.clone())
                    .value(column.clone(), renamed)
                    .and_where(Expr::col(id.clone()).eq(row_id))
                    .to_owned(),
            )
            .await?;
        }
    }
    Ok(())
}

/// In the tags of a template (`{{ … }}` and `{% … %}`), and nowhere else, give the variable
/// `from` the name `to`.
fn rename_in_tags(template: &str, from: &str, to: &str) -> String {
    let mut renamed = String::with_capacity(template.len());
    let mut rest = template;
    loop {
        let open = [("{{", "}}"), ("{%", "%}")]
            .into_iter()
            .filter_map(|(open, close)| Some((rest.find(open)?, close)))
            .min();
        let Some((start, close)) = open else {
            return renamed + rest;
        };
        let end = rest[start..]
            .find(close)
            .map_or(rest.len(), |at| start + at + close.len());
        renamed.push_str(&rest[..start]);
        renamed.push_str(&rename_word(&rest[start..end], from, to));
        rest = &rest[end..];
    }
}

/// Turn the word `from` into `to`. A longer name that holds it stays, and so does a field
/// of that name, such as `item.skill`.
fn rename_word(text: &str, from: &str, to: &str) -> String {
    let in_a_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut renamed = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let after = &rest[at + from.len()..];
        let longer = rest[..at]
            .chars()
            .next_back()
            .is_some_and(|c| in_a_name(c) || c == '.')
            || after.chars().next().is_some_and(in_a_name);
        renamed.push_str(&rest[..at]);
        renamed.push_str(if longer { from } else { to });
        rest = after;
    }
    renamed + rest
}
