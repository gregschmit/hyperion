#![allow(clippy::missing_errors_doc)]
use axum::Extension;
use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    initializers::access::{self, Guest},
    models::{runs, schedules},
    views,
};

/// How many runs the page lists.
const RUNS: u64 = 200;

/// The queue and the history. It's the home page too.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let recent = runs::Model::recent(&ctx.db, RUNS).await?;
    let rows: Vec<_> = recent.iter().map(views::run).collect();
    // A new m8r has nothing to list, so the page says how to set it up.
    let setup = rows.is_empty() && schedules::Model::list(&ctx.db).await?.is_empty();
    let data = json!({ "runs": rows, "setup": setup });
    views::page(&v, &ctx, "runs/index.html", "runs", data).await
}

/// One run: its command, what it printed, and the prompt it got. A guest, who holds the run's
/// share link, gets the same and nothing more of m8r. Anyone else gets a share link to hand
/// on.
#[debug_handler]
pub async fn show(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    guest: Option<Extension<Guest>>,
) -> Result<Response> {
    let run = runs::Model::find(&ctx.db, id).await?;
    let mut data = json!({
        "run": views::run(&run),
        "output_html": views::markdown::render(&run.output),
    });
    if guest.is_some() {
        return views::guest_page(&v, "runs/show.html", data);
    }
    let (url, until) = access::share_link(&ctx, run.id, access::SHARE_LIMIT.as_secs())?;
    data["share"] = json!({ "url": url, "until": views::time(&until.into()) });
    views::page(&v, &ctx, "runs/show.html", "runs", data).await
}

pub fn routes() -> Routes {
    Routes::new()
        .add("/", get(index))
        .add("/runs", get(index))
        .add("/runs/{id}", get(show))
}
