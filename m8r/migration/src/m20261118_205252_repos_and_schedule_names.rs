use loco_rs::schema::{drop_table, rename_column, table_auto_tz};
use sea_orm_migration::{prelude::*, schema::*};

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
        let db = m.get_connection();

        // m8r knows the repos that it works in. A repo is a clone, and the project on its
        // forge, which says which watchlist items are the repo's. Blank: m8r doesn't know the
        // project, and reads it from the clone's `origin`.
        m.create_table(
            table_auto_tz(Alias::new("repos"))
                .col(big_pk_auto(Alias::new("id")))
                .col(string_uniq(Alias::new("path")))
                .col(string(Alias::new("url")).default(""))
                .to_owned(),
        )
        .await?;
        // A project has one repo. A partial index, which sea-query can't build.
        db.execute_unprepared(
            "CREATE UNIQUE INDEX idx_repos_one_url ON repos (url) WHERE url <> ''",
        )
        .await?;

        // A watchlist item is in a repo, and a schedule runs in repos. The rows of before name
        // no repo, so both tables start new, and empty. `repo_id` references repos, ON DELETE
        // RESTRICT: a repo that has items stays.
        drop_table(m, "watchlist_items").await?;
        m.create_table(
            table_auto_tz(Alias::new("watchlist_items"))
                .col(big_pk_auto(Alias::new("id")))
                .col(string_uniq(Alias::new("url")))
                .col(big_integer(Alias::new("repo_id")))
                // What the URL says it is: `issues`, or `pull/merge requests`.
                .col(string(Alias::new("category")))
                .col(string(Alias::new("priority")).default("medium"))
                .col(string(Alias::new("note")).default(""))
                // Someone asked you for it: a review, if it's a PR/MR.
                .col(boolean(Alias::new("requested")).default(false))
                // A copy of the forge's title, for the pages.
                .col(string(Alias::new("title")).default(""))
                .col(timestamp_with_time_zone_null(Alias::new("checked_at")))
                .foreign_key(
                    ForeignKey::create()
                        .name("fk-watchlist_items-repo_id-to-repos")
                        .from(Alias::new("watchlist_items"), Alias::new("repo_id"))
                        .to(Alias::new("repos"), Alias::new("id"))
                        .on_delete(ForeignKeyAction::Restrict)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        // A schedule has a name of its own. It takes its repos' items of one category, or of
        // all, or none, and it can take only the first of them. The runs of the schedules of
        // before stay in the history, with no schedule (ON DELETE SET NULL).
        drop_table(m, "schedules").await?;
        drop_table(m, "categories").await?;
        m.create_table(
            table_auto_tz(Alias::new("schedules"))
                .col(big_pk_auto(Alias::new("id")))
                .col(string_uniq(Alias::new("name")))
                // Blank: no items. Or `all`, or a category.
                .col(string(Alias::new("category")).default(""))
                // 0: all of them.
                .col(integer(Alias::new("max_items")).default(0))
                .col(string(Alias::new("hron")))
                .col(string(Alias::new("command")).default(""))
                .col(text(Alias::new("prompt")).default(""))
                .to_owned(),
        )
        .await?;
        // `schedule_id` references schedules, ON DELETE CASCADE, and `repo_id` references
        // repos, ON DELETE RESTRICT: a repo that a schedule runs in stays.
        m.create_table(
            Table::create()
                .table(Alias::new("schedule_repos"))
                .col(big_integer(Alias::new("schedule_id")))
                .col(big_integer(Alias::new("repo_id")))
                .primary_key(
                    Index::create()
                        .col(Alias::new("schedule_id"))
                        .col(Alias::new("repo_id")),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk-schedule_repos-schedule_id-to-schedules")
                        .from(Alias::new("schedule_repos"), Alias::new("schedule_id"))
                        .to(Alias::new("schedules"), Alias::new("id"))
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk-schedule_repos-repo_id-to-repos")
                        .from(Alias::new("schedule_repos"), Alias::new("repo_id"))
                        .to(Alias::new("repos"), Alias::new("id"))
                        .on_delete(ForeignKeyAction::Restrict)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        // A run keeps the name that its schedule had, and knows its repo. The history outlives
        // both: `repo_id` references repos, ON DELETE SET NULL, and `repo` is the clone's path.
        // An error of a schedule of before has no run left that could clear it.
        rename_column(m, "runs", "operation", "name").await?;
        db.execute_unprepared(
            "ALTER TABLE runs ADD COLUMN repo_id bigint REFERENCES repos (id) ON DELETE SET NULL;
             UPDATE errors SET cleared_at = strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')
               WHERE errors.cleared_at IS NULL AND errors.subject LIKE 'schedule %';",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, _m: &SchemaManager) -> Result<(), DbErr> {
        // The schedules and the watchlist of before are gone.
        Err(DbErr::Migration(
            "This migration can't be taken back. Restore the database file from before it."
                .to_string(),
        ))
    }
}
