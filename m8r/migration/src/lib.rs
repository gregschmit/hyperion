#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
pub use sea_orm_migration::prelude::*;
use std::path::Path;

pub mod schema;

mod m20261113_221654_init;
mod m20261116_193236_watchlist_prompts_and_api;
mod m20261116_230607_errors_and_prompt_names;
mod m20261118_205252_repos_and_schedule_names;
mod m20261119_050100_drop_configurations;
mod m20261119_054135_add_enabled_to_schedules;
mod m20261119_064819_rename_hron_to_when_on_schedules;
mod m20261119_070730_drop_artifacts;
pub struct Migrator;

/// The same migrations, with the trait's own `up`. `Migrator::up` runs that one and then does
/// one thing more, and a method that replaces a trait's default can't call it on its own type.
struct Migrations;

#[async_trait::async_trait]
impl MigratorTrait for Migrations {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261113_221654_init::Migration),
            Box::new(m20261116_193236_watchlist_prompts_and_api::Migration),
            Box::new(m20261116_230607_errors_and_prompt_names::Migration),
            Box::new(m20261118_205252_repos_and_schedule_names::Migration),
            Box::new(m20261119_050100_drop_configurations::Migration),
            Box::new(m20261119_054135_add_enabled_to_schedules::Migration),
            Box::new(m20261119_064819_rename_hron_to_when_on_schedules::Migration),
            Box::new(m20261119_070730_drop_artifacts::Migration),
            // inject-above (do not remove this comment)
        ]
    }
}

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        Migrations::migrations()
    }

    /// Apply what's pending. Then, where m8r keeps `schema.sql`, write it: see `schema`.
    async fn up<'c, C>(db: C, steps: Option<u32>) -> Result<(), DbErr>
    where
        C: IntoSchemaManagerConnection<'c>,
    {
        Migrations::up(db, steps).await?;
        if schema::is_kept() {
            schema::write(Path::new(schema::FILE)).await?;
        }
        Ok(())
    }
}
