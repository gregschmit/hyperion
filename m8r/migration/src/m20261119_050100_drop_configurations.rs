use loco_rs::schema::drop_table;
use sea_orm_migration::prelude::*;

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
        // A schedule's prompt is all that a run gets, so m8r keeps no text for every prompt,
        // and needs none to run.
        drop_table(m, "configurations").await?;
        // No tick asks for a configuration any more, so nothing else would clear its error.
        m.get_connection()
            .execute_unprepared(
                "UPDATE errors SET cleared_at = strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')
                   WHERE errors.cleared_at IS NULL AND errors.subject = 'configuration'",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _m: &SchemaManager) -> Result<(), DbErr> {
        // The configurations are gone.
        Err(DbErr::Migration(
            "This migration can't be taken back. Restore the database file from before it."
                .to_string(),
        ))
    }
}
