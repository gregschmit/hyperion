#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;

use super::saved;
use crate::{
    models::repos::{Changes, Model, Params},
    settings::Settings,
    views::api::repo,
};

#[debug_handler]
pub async fn list(State(ctx): State<AppContext>) -> Result<Response> {
    let repos = Model::list(&ctx.db).await?;
    format::json(repos.iter().map(repo).collect::<Vec<_>>())
}

#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    format::json(repo(&Model::find(&ctx.db, id).await?))
}

/// Add a repo: a clone that's there. With no URL, m8r reads the project from the clone.
#[debug_handler]
pub async fn create(State(ctx): State<AppContext>, Json(params): Json<Params>) -> Result<Response> {
    let shell = Settings::from_context(&ctx)?.shell;
    saved(Model::create(&ctx.db, &shell, &params).await?, repo)
}

#[debug_handler]
pub async fn update(
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Json(changes): Json<Changes>,
) -> Result<Response> {
    let shell = Settings::from_context(&ctx)?.shell;
    saved(Model::change(&ctx.db, &shell, id, changes).await?, repo)
}

/// Remove it, unless a watchlist item is in it or a schedule runs in it.
#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let found = Model::find(&ctx.db, id).await?;
    match found.removed(&ctx.db).await? {
        Ok(()) => format::empty_json(),
        Err(problem) => bad_request(problem),
    }
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/repos")
        .add("/", get(list))
        .add("/", post(create))
        .add("/{id}", get(show))
        .add("/{id}", patch(update))
        .add("/{id}", delete(remove))
}
