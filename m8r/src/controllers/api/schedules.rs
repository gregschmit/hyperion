#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;

use super::saved;
use crate::{
    models::{
        artifacts, runs,
        schedules::{Changes, Model, Params},
    },
    settings::Settings,
    views::api,
};

#[debug_handler]
pub async fn list(State(ctx): State<AppContext>) -> Result<Response> {
    let schedules = Model::list(&ctx.db).await?;
    format::json(schedules.iter().map(api::schedule).collect::<Vec<_>>())
}

#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    format::json(api::schedule(&Model::find(&ctx.db, id).await?))
}

#[debug_handler]
pub async fn create(State(ctx): State<AppContext>, Json(params): Json<Params>) -> Result<Response> {
    saved(Model::create(&ctx.db, &params).await?, api::schedule)
}

#[debug_handler]
pub async fn update(
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Json(changes): Json<Changes>,
) -> Result<Response> {
    saved(Model::change(&ctx.db, id, changes).await?, api::schedule)
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    Model::remove(&ctx.db, id).await?;
    format::empty_json()
}

/// Queue the schedule's runs now. The answer is the runs that wait: one for each repo.
#[debug_handler]
pub async fn run(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let (schedule, _) = Model::find(&ctx.db, id).await?;
    let pages = Settings::from_context(&ctx)?.url;
    let mut queued = Vec::new();
    for run in runs::Model::queue(&ctx.db, &schedule).await? {
        let reported = artifacts::Model::of_run(&ctx.db, run.id).await?;
        queued.push(api::run(&run, &reported, &pages));
    }
    format::json(queued)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/schedules")
        .add("/", get(list))
        .add("/", post(create))
        .add("/{id}", get(show))
        .add("/{id}", patch(update))
        .add("/{id}", delete(remove))
        .add("/{id}/run", post(run))
}
