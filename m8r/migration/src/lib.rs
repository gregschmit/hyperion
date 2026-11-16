#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
pub use sea_orm_migration::prelude::*;

mod m20261113_221654_init;
mod m20261116_193236_watchlist_prompts_and_api;
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261113_221654_init::Migration),
            Box::new(m20261116_193236_watchlist_prompts_and_api::Migration),
            // inject-above (do not remove this comment)
        ]
    }
}
