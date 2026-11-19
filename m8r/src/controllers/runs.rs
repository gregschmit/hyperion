#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    models::{artifacts, runs, schedules},
    views,
};

/// How many runs the page lists.
const RUNS: u64 = 200;

/// The queue and the history, with what each run reported. It's the home page too.
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
    // A new m8r has nothing to list, so the page says how to set it up.
    let setup = rows.is_empty() && schedules::Model::list(&ctx.db).await?.is_empty();
    let data = json!({ "runs": rows, "setup": setup });
    views::page(&v, &ctx, "runs/index.html", "runs", data).await
}

/// One run: its command, what it reported, what it printed, and the prompt it got.
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
        .add("/", get(index))
        .add("/runs", get(index))
        .add("/runs/{id}", get(show))
}
