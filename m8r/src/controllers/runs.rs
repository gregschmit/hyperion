#![allow(clippy::missing_errors_doc)]
use axum::Extension;
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    initializers::access::{self, Caller},
    models::{runs, schedules},
    settings::Settings,
    stream::Printed,
    views,
};

/// How many runs the page lists.
const RUNS: u64 = 200;

/// The queue and the history. It's the home page too.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
) -> Result<Response> {
    let recent = runs::Model::recent(&ctx.db, RUNS).await?;
    let rows: Vec<_> = recent.iter().map(views::run).collect();
    // A new m8r has nothing to list, so the page says how to set it up.
    let setup = rows.is_empty() && schedules::Model::list(&ctx.db).await?.is_empty();
    let data = json!({ "runs": rows, "setup": setup });
    views::page(&v, &ctx, &caller, "runs/index.html", "runs", data).await
}

/// What a run's page is asked for: what its address says after the `?`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Shown {
    /// `output`: only the part of the page that has what the run printed. The page's script
    /// asks for it again and again while the run is in progress. Anything else: the page.
    pub part: String,
}

/// What a run printed, for its page. A run that's over has its report in its row. For one in
/// progress, it's what its command has printed so far: an agent's steps, or text.
fn printed(ctx: &AppContext, run: &runs::Model) -> Result<Value> {
    if run.is_over() {
        let html = views::markdown::render(&run.output);
        return Ok(json!({ "live": false, "output_html": html }));
    }
    let file = Settings::from_context(ctx)?.output_file(run.id);
    let (steps_html, text) = match Printed::read_file(&file) {
        Some(Printed::Stream(stream)) => {
            (views::markdown::render(&stream.markdown()), String::new())
        }
        Some(Printed::Text(text)) => (String::new(), text),
        None => (String::new(), String::new()),
    };
    Ok(json!({ "live": true, "steps_html": steps_html, "text": text }))
}

/// One run: its command, what it printed, and the prompt it got. A guest, who holds the run's
/// share link, gets the same and nothing more of m8r. Anyone else gets a share link to hand
/// on.
#[debug_handler]
pub async fn show(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
    Query(shown): Query<Shown>,
) -> Result<Response> {
    let run = runs::Model::find(&ctx.db, id).await?;
    let mut data = json!({
        "run": views::run(&run),
        "printed": printed(&ctx, &run)?,
    });
    if shown.part == "output" {
        data["shared"] = json!(caller.is_guest());
        return format::render().view(&v, "runs/output.html", data);
    }
    if caller.is_guest() {
        return views::guest_page(&v, "runs/show.html", data);
    }
    let (url, until) = access::share_link(&ctx, run.id, access::SHARE_LIMIT.as_secs())?;
    data["share"] = json!({ "url": url, "until": views::time(&until.into()) });
    views::page(&v, &ctx, &caller, "runs/show.html", "runs", data).await
}

pub fn routes() -> Routes {
    Routes::new()
        .add("/", get(index))
        .add("/runs", get(index))
        .add("/runs/{id}", get(show))
}
