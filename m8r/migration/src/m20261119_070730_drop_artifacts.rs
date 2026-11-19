use loco_rs::schema::drop_table;
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
        // A run's report is what its command printed, with the links to what it made. So m8r
        // keeps no list of artifacts, and hosts no files.
        drop_table(m, "artifacts").await?;
        Ok(())
    }

    async fn down(&self, _m: &SchemaManager) -> Result<(), DbErr> {
        // The artifacts are gone.
        Err(DbErr::Migration(
            "This migration can't be taken back. Restore the database file from before it."
                .to_string(),
        ))
    }
}
