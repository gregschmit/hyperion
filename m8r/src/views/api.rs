//! What the API and the MCP tools give back. An entity never goes out as it is.

use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::models::{artifacts, categories, configurations, runs, schedules, watchlist_items};

fn time(at: Option<&DateTimeWithTimeZone>) -> Option<String> {
    at.map(DateTimeWithTimeZone::to_rfc3339)
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
        "category": category.name,
        "priority": item.priority,
        "note": item.note,
        "review_requested": item.review_requested,
        // The forge's title, once m8r has asked for it.
        "title": item.title,
        "checked_at": time(item.checked_at.as_ref()),
    })
}

#[must_use]
pub fn schedule((schedule, category): &schedules::Listed) -> Value {
    let next = schedule
        .next(&jiff::Zoned::now())
        .map(|at| at.timestamp().to_string());
    json!({
        "id": schedule.id,
        "operation": schedule.operation,
        "category": category.as_ref().map(|it| it.name.as_str()).unwrap_or_default(),
        "repos": schedule.repos().collect::<Vec<_>>(),
        "hron": schedule.times().collect::<Vec<_>>(),
        "command": schedule.command,
        "prompt": schedule.prompt,
        // The first of its times to come.
        "next": next,
    })
}

#[must_use]
pub fn configuration(configuration: &configurations::Model) -> Value {
    json!({
        "id": configuration.id,
        "name": configuration.name,
        "active": configuration.active,
        "prompt": configuration.prompt,
    })
}

/// A run in a list: all but its prompt and what it printed, which are long.
#[must_use]
pub fn run(run: &runs::Model, artifacts: &[artifacts::Model], pages: &str) -> Value {
    let artifacts: Vec<Value> = artifacts
        .iter()
        .map(|artifact| {
            // A hosted artifact's link is its page in m8r, which wants a session.
            let url = if artifact.hosted() {
                format!("{}/artifacts/{}", pages.trim_end_matches('/'), artifact.id)
            } else {
                artifact.url.clone()
            };
            json!({
                "id": artifact.id,
                "name": artifact.name,
                "url": url,
                "hosted": artifact.hosted(),
            })
        })
        .collect();
    json!({
        "id": run.id,
        "schedule_id": run.schedule_id,
        "operation": run.operation,
        "repo": run.repo,
        "command": run.command,
        "status": run.status,
        "error": run.error,
        "queued_at": time(Some(&run.created_at)),
        "started_at": time(run.started_at.as_ref()),
        "finished_at": time(run.finished_at.as_ref()),
        "artifacts": artifacts,
    })
}

/// One run, whole.
#[must_use]
pub fn run_whole(run: &runs::Model, artifacts: &[artifacts::Model], pages: &str) -> Value {
    let mut whole = self::run(run, artifacts, pages);
    whole["prompt"] = json!(run.prompt);
    whole["output"] = json!(run.output);
    whole
}
