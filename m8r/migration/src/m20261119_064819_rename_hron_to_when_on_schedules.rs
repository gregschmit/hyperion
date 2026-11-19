use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// All of it happens, or none of it.
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // The column says what it holds: when the schedule runs. SQL has a word `when` of its
        // own, so a statement that's written by hand quotes the name.
        rename_column(m, "schedules", "hron", "when").await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        rename_column(m, "schedules", "when", "hron").await?;
        Ok(())
    }
}
