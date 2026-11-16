//! What the pages show. The templates are in `assets/views`.

pub mod api;

use std::time::Duration;

use chrono::{DateTime, Local};
use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::{
    models::{artifacts, categories, runs, schedules, tool_errors, watchlist_items},
    settings::Settings,
};

/// The scheduler ticks every minute. After this long with no tick, it isn't running.
const TICK_LATE: Duration = Duration::from_secs(150);

/// A time as the pages show it: in the server's time zone.
#[must_use]
pub fn time(at: &DateTimeWithTimeZone) -> String {
    at.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn time_or_blank(at: Option<&DateTimeWithTimeZone>) -> String {
    at.map(time).unwrap_or_default()
}

/// Render a page. Every page also shows the tools that don't work, so `data` gets them.
///
/// # Errors
/// When a query or the template fails.
pub async fn page(
    v: &TeraView,
    ctx: &AppContext,
    key: &str,
    section: &str,
    mut data: Value,
) -> Result<Response> {
    let errors: Vec<Value> = tool_errors::Model::list(&ctx.db)
        .await?
        .iter()
        .map(|error| {
            json!({
                "tool": error.tool,
                "host": error.host,
                "message": error.message,
                "since": time(&error.created_at),
            })
        })
        .collect();
    let last_tick = std::fs::metadata(Settings::from_context(ctx)?.tick_file())
        .and_then(|file| file.modified())
        .ok();
    let scheduler_late = last_tick.is_none_or(|at| at.elapsed().is_ok_and(|age| age > TICK_LATE));
    data["section"] = json!(section);
    data["has_errors"] = json!(scheduler_late || !errors.is_empty());
    data["tool_errors"] = json!(errors);
    data["scheduler_late"] = json!(scheduler_late);
    data["last_tick"] = json!(last_tick
        .map(|at| DateTime::<Local>::from(at)
            .format("%Y-%m-%d %H:%M")
            .to_string())
        .unwrap_or_default());
    format::render().view(v, key, data)
}

#[must_use]
pub fn artifact(artifact: &artifacts::Model) -> Value {
    let href = if artifact.hosted() {
        format!("/artifacts/{}", artifact.id)
    } else {
        artifact.url.clone()
    };
    json!({
        "id": artifact.id,
        "name": artifact.name,
        "hosted": artifact.hosted(),
        "href": href,
    })
}

#[must_use]
pub fn run(run: &runs::Model, artifacts: &[artifacts::Model]) -> Value {
    json!({
        "id": run.id,
        "operation": run.operation,
        "repo": run.repo,
        "command": run.command,
        "status": run.status,
        "queued_at": time(&run.created_at),
        "started_at": time_or_blank(run.started_at.as_ref()),
        "finished_at": time_or_blank(run.finished_at.as_ref()),
        "prompt": run.prompt,
        "output": run.output,
        "error": run.error,
        "artifacts": artifacts.iter().map(artifact).collect::<Vec<_>>(),
    })
}

#[must_use]
pub fn category(category: &categories::Model) -> Value {
    json!({ "id": category.id, "name": category.name })
}

#[must_use]
pub fn watchlist_item((item, category): &watchlist_items::Listed) -> Value {
    json!({
        "id": item.id,
        "url": item.url,
        "label": item.label(),
        "title": item.title,
        "category": category.name,
        "priority": item.priority,
        "note": item.note,
        "reviewable": item.reviewable(),
        "review_requested": item.review_requested,
        "checked_at": time_or_blank(item.checked_at.as_ref()),
    })
}

#[must_use]
pub fn schedule((schedule, category): &schedules::Listed) -> Value {
    // On the server's clock, like every time on the pages.
    let next = schedule
        .next(&jiff::Zoned::now())
        .map(|at| at.strftime("%a %Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    json!({
        "id": schedule.id,
        "operation": schedule.operation,
        "category": category.as_ref().map(|it| it.name.as_str()).unwrap_or_default(),
        "repo": schedule.repo,
        "hron": schedule.hron,
        "command": schedule.command,
        "next": next,
        // A time from before m8r read times in words may be one that it can't read.
        "unreadable": schedules::parse(&schedule.hron).is_err(),
    })
}
