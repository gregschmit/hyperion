#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::{
    models::{artifacts, runs::Model},
    settings::Settings,
    views::api,
};

/// How many runs a list holds, unless it asks for fewer.
pub const RUNS: u64 = 50;

#[derive(Debug, Deserialize)]
pub struct Filter {
    pub limit: Option<u64>,
}

/// The latest runs, newest first: the queue and the history.
#[debug_handler]
pub async fn list(State(ctx): State<AppContext>, Query(filter): Query<Filter>) -> Result<Response> {
    let pages = Settings::from_context(&ctx)?.url;
    let limit = filter.limit.unwrap_or(RUNS).min(RUNS);
    let mut runs = Vec::new();
    for run in Model::recent(&ctx.db, limit).await? {
        let reported = artifacts::Model::of_run(&ctx.db, run.id).await?;
        runs.push(api::run(&run, &reported, &pages));
    }
    format::json(runs)
}

/// One run, with its prompt and what it printed.
#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let pages = Settings::from_context(&ctx)?.url;
    let run = Model::find(&ctx.db, id).await?;
    let reported = artifacts::Model::of_run(&ctx.db, run.id).await?;
    format::json(api::run_whole(&run, &reported, &pages))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/runs")
        .add("/", get(list))
        .add("/{id}", get(show))
}
