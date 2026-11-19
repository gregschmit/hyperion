//! What the API and the MCP tools give back. An entity never goes out as it is.

use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::models::{artifacts, repos, runs, schedules, watchlist_items};

fn time(at: Option<&DateTimeWithTimeZone>) -> Option<String> {
    at.map(DateTimeWithTimeZone::to_rfc3339)
}

#[must_use]
pub fn repo(repo: &repos::Model) -> Value {
    json!({ "id": repo.id, "path": repo.path, "url": repo.url })
}

#[must_use]
pub fn watchlist_item((item, repo): &watchlist_items::Listed) -> Value {
    json!({
        "id": item.id,
        "url": item.url,
        "repo_id": repo.id,
        // The path of the repo's clone.
        "repo": repo.path,
        "category": item.category,
        "priority": item.priority,
        "note": item.note,
        "requested": item.requested,
        // The forge's title, once m8r has asked for it.
        "title": item.title,
        "checked_at": time(item.checked_at.as_ref()),
    })
}

#[must_use]
pub fn schedule((schedule, repos): &schedules::Listed) -> Value {
    let next = schedule
        .next(&jiff::Zoned::now())
        .map(|at| at.timestamp().to_string());
    json!({
        "id": schedule.id,
        "name": schedule.name,
        "category": schedule.category,
        "max_items": schedule.max_items,
        // The paths of the repos' clones.
        "repos": repos.iter().map(|repo| repo.path.as_str()).collect::<Vec<_>>(),
        "hron": schedule.times().collect::<Vec<_>>(),
        "enabled": schedule.enabled,
        "command": schedule.command,
        "prompt": schedule.prompt,
        // The first of its times to come. None while it's off.
        "next": next,
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
        "name": run.name,
        "repo_id": run.repo_id,
        "repo": run.repo,
        "command": run.command,
        "status": run.status,
        "error": run.error,
        "created_at": time(Some(&run.created_at)),
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
