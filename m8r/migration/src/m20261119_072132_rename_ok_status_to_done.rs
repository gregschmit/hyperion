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
        // A run whose command ended well is done. `ok` said more: that someone found its work
        // good, and nobody looked.
        m.get_connection()
            .execute_unprepared("UPDATE runs SET status = 'done' WHERE runs.status = 'ok'")
            .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.get_connection()
            .execute_unprepared("UPDATE runs SET status = 'ok' WHERE runs.status = 'done'")
            .await?;
        Ok(())
    }
}
