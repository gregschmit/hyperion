#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    models::{artifacts, runs},
    views,
};

/// How many runs the page lists.
const RUNS: u64 = 200;

/// The queue and the history.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let mut rows = Vec::new();
    for run in runs::Model::recent(&ctx.db, RUNS).await? {
        let reported = artifacts::Model::of_run(&ctx.db, run.id).await?;
        rows.push(views::run(&run, &reported));
    }
    views::page(&v, &ctx, "runs/index.html", "runs", json!({ "runs": rows })).await
}

/// One run: the prompt it got, what the agent printed, and what it reported.
#[debug_handler]
pub async fn show(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
) -> Result<Response> {
    let run = runs::Model::find(&ctx.db, id).await?;
    let reported = artifacts::Model::of_run(&ctx.db, run.id).await?;
    let data = json!({ "run": views::run(&run, &reported) });
    views::page(&v, &ctx, "runs/show.html", "runs", data).await
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/runs")
        .add("/", get(index))
        .add("/{id}", get(show))
}
