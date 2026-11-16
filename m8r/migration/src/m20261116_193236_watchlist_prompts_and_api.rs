use loco_rs::schema::{
    add_column, create_table, drop_table, remove_column, rename_column, table_auto_tz, ColType,
};
use sea_orm_migration::{prelude::*, schema::*};

/// The categories that every m8r starts with.
const CATEGORIES: [&str; 2] = ["Issue", "Pull/Merge Request"];

/// The prompts that a configuration had: its own, and one for each operation.
const PROMPTS: [&str; 4] = [
    "system_message",
    "triage_prompt",
    "review_prompt",
    "develop_prompt",
];

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();

        create_table(
            m,
            "categories",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::StringLenUniq(255)),
            ],
            &[],
        )
        .await?;
        for name in CATEGORIES {
            m.exec_stmt(
                Query::insert()
                    .into_table(Alias::new("categories"))
                    .columns([Alias::new("name")])
                    .values_panic([name.into()])
                    .to_owned(),
            )
            .await?;
        }

        // The items are the watchlist's items, and each has a category. SQLite takes a foreign
        // key only when it makes the table, so the items move to a new one. `category_id`
        // references categories, ON DELETE RESTRICT: a category that has items stays.
        m.create_table(
            table_auto_tz(Alias::new("watchlist_items"))
                .col(big_pk_auto(Alias::new("id")))
                .col(string_uniq(Alias::new("url")))
                .col(big_integer(Alias::new("category_id")))
                .col(string(Alias::new("priority")).default("medium"))
                .col(string(Alias::new("note")).default(""))
                // Someone asked you to review it. Only a PR/MR has that.
                .col(boolean(Alias::new("review_requested")).default(false))
                // A copy of the forge's title, for the pages.
                .col(string(Alias::new("title")).default(""))
                .col(timestamp_with_time_zone_null(Alias::new("checked_at")))
                .foreign_key(
                    ForeignKey::create()
                        .name("fk-watchlist_items-category_id-to-categories")
                        .from(Alias::new("watchlist_items"), Alias::new("category_id"))
                        .to(Alias::new("categories"), Alias::new("id"))
                        .on_delete(ForeignKeyAction::Restrict)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        // An item's URL says which of the two it is.
        db.execute_unprepared(
            "INSERT INTO watchlist_items
               (created_at, updated_at, url, category_id, priority, note, title, checked_at)
             SELECT created_at, updated_at, url,
               (SELECT id FROM categories WHERE name =
                 CASE WHEN url LIKE '%/pull/%' OR url LIKE '%/merge_requests/%'
                   THEN 'Pull/Merge Request' ELSE 'Issue' END),
               priority, note, title, checked_at
             FROM items ORDER BY id",
        )
        .await?;
        drop_table(m, "items").await?;

        // A schedule says when in words, and hron reads the cron expressions that are there.
        rename_column(m, "schedules", "cron", "hron").await?;
        // A schedule has a command of its own, and a run keeps a copy. Until now, each run
        // started the agent's command from the settings: production has it in M8R_AGENT, and
        // development's is `cat`.
        let agent = std::env::var("M8R_AGENT").unwrap_or_else(|_| "cat".to_string());
        for table in ["schedules", "runs"] {
            add_column(
                m,
                table,
                "command",
                ColType::StringWithDefault(String::new()),
            )
            .await?;
            m.exec_stmt(
                Query::update()
                    .table(Alias::new(table))
                    .value(Alias::new("command"), agent.as_str())
                    .to_owned(),
            )
            .await?;
        }

        // A schedule has a prompt of its own too. Each one gets its operation's prompt from the
        // active configuration, or with none active, from the one that changed last. The
        // templates read the list by its new name first.
        rename_in_prompts(m, "items", "watchlist_items").await?;
        add_column(
            m,
            "schedules",
            "prompt",
            ColType::TextWithDefault(String::new()),
        )
        .await?;
        db.execute_unprepared(
            "UPDATE schedules SET prompt = COALESCE((
               SELECT CASE schedules.operation
                 WHEN 'triage' THEN triage_prompt
                 WHEN 'review' THEN review_prompt
                 WHEN 'develop' THEN develop_prompt
                 ELSE '' END
               FROM configurations ORDER BY active DESC, updated_at DESC LIMIT 1), '')",
        )
        .await?;
        for prompt in &PROMPTS[1..] {
            remove_column(m, "configurations", prompt).await?;
        }

        // A schedule's runs get the items of one category, or with none, no items. A category
        // that a schedule uses stays. The helpers add no foreign key to a table that's there.
        db.execute_unprepared(
            "ALTER TABLE schedules ADD COLUMN category_id bigint
               REFERENCES categories (id) ON DELETE RESTRICT",
        )
        .await?;
        // Until now, a run got every item. So that a schedule's runs go on with a list, each
        // schedule gets the category that fits its operation.
        db.execute_unprepared(
            "UPDATE schedules SET category_id = (SELECT id FROM categories WHERE name =
               CASE schedules.operation
                 WHEN 'review' THEN 'Pull/Merge Request'
                 WHEN 'triage' THEN 'Issue'
                 WHEN 'develop' THEN 'Issue' END)",
        )
        .await?;

        // A run takes its command and its prompt from its schedule when it starts. The history
        // outlives the schedule. A run that waits now gets the schedule that queued it.
        db.execute_unprepared(
            "ALTER TABLE runs ADD COLUMN schedule_id bigint
               REFERENCES schedules (id) ON DELETE SET NULL",
        )
        .await?;
        db.execute_unprepared(
            "UPDATE runs SET schedule_id = (
               SELECT id FROM schedules
               WHERE operation = runs.operation AND repo = runs.repo ORDER BY id LIMIT 1)
             WHERE status = 'queued'",
        )
        .await?;

        // What the API and the MCP endpoint take in place of a session. m8r keeps a token's
        // SHA-256, not the token.
        create_table(
            m,
            "tokens",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::StringUniq),
                ("digest", ColType::StringUniq),
                // The resources that the token covers, or `all`.
                ("scope", ColType::String),
            ],
            &[],
        )
        .await
    }

    async fn down(&self, _m: &SchemaManager) -> Result<(), DbErr> {
        // The prompts moved from the configurations to the schedules, and SQLite can't drop a
        // column that's a foreign key.
        Err(DbErr::Migration(
            "This migration can't be taken back. Restore the database file from before it."
                .to_string(),
        ))
    }
}

/// Rename the list that the configurations' templates loop over: `{% for item in items %}`.
async fn rename_in_prompts(m: &SchemaManager<'_>, from: &str, to: &str) -> Result<(), DbErr> {
    let table = Alias::new("configurations");
    let configurations = m
        .get_connection()
        .query_all(
            Query::select()
                .column(Alias::new("id"))
                .columns(PROMPTS.map(Alias::new))
                .from(table.clone()),
        )
        .await?;
    for configuration in configurations {
        let id: i64 = configuration.try_get("", "id")?;
        let mut renamed = Query::update();
        renamed
            .table(table.clone())
            .and_where(Expr::col(Alias::new("id")).eq(id));
        for prompt in PROMPTS {
            let text: String = configuration.try_get("", prompt)?;
            renamed.value(Alias::new(prompt), rename_in_tags(&text, from, to));
        }
        m.exec_stmt(renamed).await?;
    }
    Ok(())
}

/// In the tags of a template (`{% … %}`), and nowhere else, turn ` in <from>` into
/// ` in <to>`.
fn rename_in_tags(template: &str, from: &str, to: &str) -> String {
    let mut renamed = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{%") {
        let end = rest[start..]
            .find("%}")
            .map_or(rest.len(), |at| start + at + 2);
        let mut tag = rest[start..end].to_string();
        // What can come after the list's name in a `for` tag.
        for after in [" ", "-", "%"] {
            tag = tag.replace(&format!(" in {from}{after}"), &format!(" in {to}{after}"));
        }
        renamed.push_str(&rest[..start]);
        renamed.push_str(&tag);
        rest = &rest[end..];
    }
    renamed + rest
}
