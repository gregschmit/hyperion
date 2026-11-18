//! What the pages show. The templates are in `assets/views`.

pub mod api;

use std::time::Duration;

use chrono::{DateTime, Local};
use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::{
    models::{artifacts, errors, repos, runs, schedules, watchlist_items},
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

/// What m8r sees whenever someone looks at a page: whether the scheduler ticks, and whether
/// m8r is set up. Each raises its error or clears it, so the page shows what's true now.
///
/// # Errors
/// When a query fails.
pub async fn look(ctx: &AppContext) -> Result<()> {
    let last_tick = std::fs::metadata(Settings::from_context(ctx)?.tick_file())
        .and_then(|file| file.modified())
        .ok();
    // A scheduler that's down can't say so. So a page raises its error, and the next tick
    // clears it.
    if last_tick.is_none_or(|at| at.elapsed().is_ok_and(|age| age > TICK_LATE)) {
        let since = last_tick.map_or_else(
            || "yet".to_string(),
            |at| {
                let at = DateTime::<Local>::from(at).format("%Y-%m-%d %H:%M");
                format!("since {at}")
            },
        );
        let message = format!(
            "The scheduler isn't running, so nothing starts a run: it ticks every minute, and \
             there was no tick {since}."
        );
        errors::Model::raise(&ctx.db, errors::SCHEDULER, &message, None).await?;
    }
    Ok(errors::Model::check_setup(&ctx.db).await?)
}

/// Render a page. Every page also shows the errors that are open, so `data` gets them.
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
    look(ctx).await?;
    let open: Vec<Value> = errors::Model::open(&ctx.db)
        .await?
        .iter()
        .map(error)
        .collect();
    data["section"] = json!(section);
    data["error_count"] = json!(open.len());
    data["open_errors"] = json!(open);
    format::render().view(v, key, data)
}

#[must_use]
pub fn error(error: &errors::Model) -> Value {
    // Where the error can be looked into, or put right.
    let href = match error.subject.as_str() {
        errors::CONFIGURATION => "/configurations".to_string(),
        errors::SCHEDULES => "/schedules".to_string(),
        subject if subject.starts_with("repo ") => "/repos".to_string(),
        _ => error
            .run_id
            .map(|id| format!("/runs/{id}"))
            .unwrap_or_default(),
    };
    json!({
        "subject": error.subject,
        "message": error.message,
        // The alert has room for a line.
        "summary": error.message.lines().next().unwrap_or_default(),
        "href": href,
        "since": time(&error.created_at),
        "open": error.cleared_at.is_none(),
        "cleared_at": time_or_blank(error.cleared_at.as_ref()),
    })
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
        "name": run.name,
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

/// A repo, and how many watchlist items and schedules it has.
#[must_use]
pub fn repo(repo: &repos::Model, (items, schedules): (u64, u64)) -> Value {
    json!({
        "id": repo.id,
        "path": repo.path,
        "url": repo.url,
        "items": items,
        "schedules": schedules,
    })
}

#[must_use]
pub fn watchlist_item((item, repo): &watchlist_items::Listed) -> Value {
    json!({
        "id": item.id,
        "url": item.url,
        "label": item.label(),
        "title": item.title,
        "repo": repo.path,
        "category": item.category,
        "priority": item.priority,
        "note": item.note,
        "requested": item.requested,
        "checked_at": time_or_blank(item.checked_at.as_ref()),
    })
}

#[must_use]
pub fn schedule((schedule, repos): &schedules::Listed) -> Value {
    // On the server's clock, like every time on the pages.
    let next = schedule
        .next(&jiff::Zoned::now())
        .map(|at| at.strftime("%a %Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    json!({
        "id": schedule.id,
        "name": schedule.name,
        "category": schedule.category,
        "max_items": schedule.max_items,
        "repos": repos.iter().map(|repo| repo.path.as_str()).collect::<Vec<_>>(),
        "hron": schedule.times().collect::<Vec<_>>(),
        "command": schedule.command,
        // The first of its times to come.
        "next": next,
        // A time from before m8r read times in words may be one that it can't read.
        "unreadable": schedule.times().any(|time| schedules::parse(time).is_err()),
    })
}
