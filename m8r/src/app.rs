use async_trait::async_trait;
use loco_rs::{
    app::{AppContext, Hooks, Initializer},
    bgworker::Queue,
    boot::{create_app, BootResult, StartMode},
    config::Config,
    controller::AppRoutes,
    db::truncate_table,
    environment::Environment,
    task::Tasks,
    Result,
};
use migration::Migrator;
use std::path::Path;

use crate::{
    controllers, initializers,
    models::{errors, repos, runs, schedule_repos, schedules, tokens, used_links, watchlist_items},
    tasks,
};

pub struct App;
#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    fn app_version() -> String {
        format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            option_env!("BUILD_SHA")
                .or(option_env!("GITHUB_SHA"))
                .unwrap_or("dev")
        )
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    async fn after_context(ctx: AppContext) -> Result<AppContext> {
        // In development, the migrator keeps `migration/schema.sql`, for you to read. The box
        // runs in its checkout and mustn't write there, and a test leaves the file to you.
        if ctx.environment == Environment::Development {
            migration::schema::keep();
        }
        Ok(ctx)
    }

    async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        Ok(vec![
            Box::new(initializers::view_engine::ViewEngineInitializer),
            // Last, so it's the outermost layer: nothing answers without a session.
            Box::new(initializers::access::AccessInitializer),
        ])
    }

    fn routes(ctx: &AppContext) -> AppRoutes {
        AppRoutes::empty() // controller routes below
            .add_route(controllers::runs::routes())
            .add_route(controllers::watchlist::routes())
            .add_route(controllers::schedules::routes())
            .add_route(controllers::repos::routes())
            .add_route(controllers::errors::routes())
            .add_route(controllers::api::watchlist_items::routes())
            .add_route(controllers::api::repos::routes())
            .add_route(controllers::api::schedules::routes())
            .add_route(controllers::api::runs::routes())
            .add_route(controllers::mcp::routes(ctx))
    }

    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        Ok(())
    }

    #[allow(unused_variables)]
    fn register_tasks(tasks: &mut Tasks) {
        tasks.register(tasks::link::Link);
        tasks.register(tasks::tick::Tick);
        tasks.register(tasks::token::Token);
        tasks.register(tasks::tokens::Tokens);
        tasks.register(tasks::revoke::Revoke);
        tasks.register(tasks::runs::Runs);
        tasks.register(tasks::output::Output);
        // tasks-inject (do not remove)
    }

    async fn truncate(ctx: &AppContext) -> Result<()> {
        truncate_table(&ctx.db, runs::Entity).await?;
        truncate_table(&ctx.db, watchlist_items::Entity).await?;
        truncate_table(&ctx.db, schedule_repos::Entity).await?;
        truncate_table(&ctx.db, schedules::Entity).await?;
        // Last of these: a repo that has items or schedules stays.
        truncate_table(&ctx.db, repos::Entity).await?;
        truncate_table(&ctx.db, tokens::Entity).await?;
        truncate_table(&ctx.db, used_links::Entity).await?;
        truncate_table(&ctx.db, errors::Entity).await?;
        Ok(())
    }

    async fn seed(_ctx: &AppContext, _base: &Path) -> Result<()> {
        Ok(())
    }
}
