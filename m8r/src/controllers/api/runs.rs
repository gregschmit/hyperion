#![allow(clippy::missing_errors_doc)]
use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream;
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::json;

use crate::{
    follow::{Change, Watch},
    models::runs::Model,
    views::{api, markdown},
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
    let limit = filter.limit.unwrap_or(RUNS).min(RUNS);
    let runs = Model::recent(&ctx.db, limit).await?;
    format::json(runs.iter().map(api::run).collect::<Vec<_>>())
}

/// One run, with its prompt and what it printed.
#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    format::json(api::run_whole(&Model::find(&ctx.db, id).await?))
}

/// What a run prints, as it comes: a stream of events (`text/event-stream`) that lasts until
/// the run is over. The run's page shows them. Each event has a name, and JSON for its data:
///
/// - `start`: what came before no longer holds, and all of it comes anew. The first event.
/// - `raw`: `text`, which is more of the text as it was written.
/// - `step`: a step that's new, or that changed. Its `id`, the step as `html`, whether it's
///   a `tool` that the agent used, and whether it's still `open`: the agent is writing it.
/// - `started`: the run, which waited, has started.
/// - `over`: the run is over. The last event.
#[debug_handler]
pub async fn output(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let run = Model::find(&ctx.db, id).await?;
    let watch = Watch::new(&ctx, &run)?;
    let events = stream::unfold(watch, |mut watch| async move {
        let change = watch.next().await?;
        Some((Ok::<_, Infallible>(event(change)), watch))
    });
    // nginx holds an answer back until it has a lot of it, unless the answer says not to.
    let headers = [("x-accel-buffering", "no")];
    let events = Sse::new(events).keep_alive(KeepAlive::default());
    Ok((headers, events).into_response())
}

/// The event that tells a page of a change.
fn event(change: Change) -> Event {
    let (name, data) = match change {
        Change::Start => ("start", json!({})),
        Change::Raw(text) => ("raw", json!({ "text": text })),
        Change::Step {
            id,
            source,
            tool,
            open,
        } => {
            let html = markdown::render(&source);
            let step = json!({ "id": id, "html": html, "tool": tool, "open": open });
            ("step", step)
        }
        Change::Started => ("started", json!({})),
        Change::Over => ("over", json!({})),
    };
    Event::default().event(name).data(data.to_string())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/runs")
        .add("/", get(list))
        .add("/{id}", get(show))
        .add("/{id}/output", get(output))
}
