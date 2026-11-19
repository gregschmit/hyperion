#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::{models::runs::Model, views::api};

/// How many runs a list holds, unless it asks for fewer.
pub const RUNS: u64 = 50;

#[derive(Debug, Deserialize)]
pub struct Filter {
    pub limit: Option<u64>,
}

/// The latest runs, newest first: the queue and the history.
#[debug_handler]
pub async fn list(State(ctx): State<AppContext>, Query(filter): Query<Filter>) -> Result<Response> {
    let limit = filter.limit.unwrap_or(RUNS).min(RUNS);
    let runs = Model::recent(&ctx.db, limit).await?;
    format::json(runs.iter().map(api::run).collect::<Vec<_>>())
}

/// One run, with its prompt and what it printed.
#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    format::json(api::run_whole(&Model::find(&ctx.db, id).await?))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/runs")
        .add("/", get(list))
        .add("/{id}", get(show))
}
