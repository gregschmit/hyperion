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
    models::{artifacts, configurations, items, runs, schedules, tool_errors, used_links},
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

    async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        Ok(vec![
            Box::new(initializers::view_engine::ViewEngineInitializer),
            // Last, so it's the outermost layer: nothing answers without a session.
            Box::new(initializers::access::AccessInitializer),
        ])
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        AppRoutes::empty() // controller routes below
            .add_route(controllers::report::routes())
            .add_route(controllers::runs::routes())
            .add_route(controllers::items::routes())
            .add_route(controllers::schedules::routes())
            .add_route(controllers::configurations::routes())
            .add_route(controllers::artifacts::routes())
    }

    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        Ok(())
    }

    #[allow(unused_variables)]
    fn register_tasks(tasks: &mut Tasks) {
        tasks.register(tasks::link::Link);
        tasks.register(tasks::tick::Tick);
        // tasks-inject (do not remove)
    }

    async fn truncate(ctx: &AppContext) -> Result<()> {
        truncate_table(&ctx.db, artifacts::Entity).await?;
        truncate_table(&ctx.db, runs::Entity).await?;
        truncate_table(&ctx.db, items::Entity).await?;
        truncate_table(&ctx.db, schedules::Entity).await?;
        truncate_table(&ctx.db, configurations::Entity).await?;
        truncate_table(&ctx.db, used_links::Entity).await?;
        truncate_table(&ctx.db, tool_errors::Entity).await?;
        Ok(())
    }

    async fn seed(_ctx: &AppContext, _base: &Path) -> Result<()> {
        Ok(())
    }
}
