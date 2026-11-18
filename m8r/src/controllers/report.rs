#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::{
    models::{artifacts, configurations, runs, schedules},
    views,
};

/// The report, which is the home page: for each schedule and each of its repos, what its
/// latest run there reported. That stays current until the schedule's next run there is over.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let mut rows: Vec<Value> = Vec::new();
    for listed in schedules::Model::list(&ctx.db).await? {
        let schedule = &listed.0;
        for repo in schedule.repos() {
            let latest = match runs::Model::latest_of(&ctx.db, schedule.id, repo).await? {
                Some(run) => {
                    let reported = artifacts::Model::of_run(&ctx.db, run.id).await?;
                    Some(views::run(&run, &reported))
                }
                None => None,
            };
            rows.push(json!({
                "schedule": views::schedule(&listed),
                "repo": repo,
                "has_run": latest.is_some(),
                "run": latest,
            }));
        }
    }
    let data = json!({
        "rows": rows,
        "unconfigured": configurations::Model::list(&ctx.db).await?.is_empty(),
    });
    views::page(&v, &ctx, "report/index.html", "report", data).await
}

pub fn routes() -> Routes {
    Routes::new().add("/", get(index))
}
