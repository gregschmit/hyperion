#![allow(clippy::missing_errors_doc)]
use chrono::{Days, Local, NaiveDate};
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::json;

use crate::{
    models::{artifacts, configurations, runs},
    views,
};

/// How far back the report looks.
const ARTIFACTS: u64 = 2000;

#[derive(Debug, Deserialize)]
pub struct Params {
    pub day: Option<NaiveDate>,
}

/// The daily report, which is the home page: the day's artifacts by run, each one a link.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Query(params): Query<Params>,
) -> Result<Response> {
    let today = Local::now().date_naive();
    let day = params.day.unwrap_or(today);

    // Newest first. A day is the server's day, which the database can't compare with.
    let reported: Vec<_> = artifacts::Model::recent(&ctx.db, ARTIFACTS)
        .await?
        .into_iter()
        .filter(|artifact| artifact.created_at.with_timezone(&Local).date_naive() == day)
        .collect();
    let mut run_ids: Vec<i64> = reported.iter().map(|artifact| artifact.run_id).collect();
    run_ids.dedup();
    let mut reports = Vec::new();
    for run_id in run_ids {
        let run = runs::Model::find(&ctx.db, run_id).await?;
        let mut of_run: Vec<_> = reported
            .iter()
            .filter(|artifact| artifact.run_id == run_id)
            .cloned()
            .collect();
        of_run.reverse();
        reports.push(views::run(&run, &of_run));
    }

    let unconfigured = configurations::Model::list(&ctx.db).await?.is_empty();
    let day_before = day.checked_sub_days(Days::new(1)).unwrap_or(day);
    let day_after = day
        .checked_add_days(Days::new(1))
        .filter(|next| *next <= today);
    let data = json!({
        "day": day.to_string(),
        "is_today": day == today,
        "day_before": day_before.to_string(),
        "has_day_after": day_after.is_some(),
        "day_after": day_after.map(|day| day.to_string()).unwrap_or_default(),
        "runs": reports,
        "unconfigured": unconfigured,
    });
    views::page(&v, &ctx, "report/index.html", "report", data).await
}

pub fn routes() -> Routes {
    Routes::new().add("/", get(index))
}
