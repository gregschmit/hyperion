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
        // A schedule can be turned off for a while: it then starts nothing at its times. The
        // schedules that are there stay on.
        add_column(m, "schedules", "enabled", ColType::BooleanWithDefault(true)).await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        remove_column(m, "schedules", "enabled").await?;
        Ok(())
    }
}
