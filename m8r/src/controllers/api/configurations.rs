#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;

use super::saved;
use crate::{
    models::configurations::{Changes, Model, Params},
    views::api::configuration,
};

#[debug_handler]
pub async fn list(State(ctx): State<AppContext>) -> Result<Response> {
    let configurations = Model::list(&ctx.db).await?;
    format::json(configurations.iter().map(configuration).collect::<Vec<_>>())
}

#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    format::json(configuration(&Model::find(&ctx.db, id).await?))
}

#[debug_handler]
pub async fn create(State(ctx): State<AppContext>, Json(params): Json<Params>) -> Result<Response> {
    saved(Model::create(&ctx.db, &params).await?, configuration)
}

#[debug_handler]
pub async fn update(
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Json(changes): Json<Changes>,
) -> Result<Response> {
    saved(Model::change(&ctx.db, id, changes).await?, configuration)
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    Model::remove(&ctx.db, id).await?;
    format::empty_json()
}

/// Make it the one active configuration.
#[debug_handler]
pub async fn activate(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let found = Model::find(&ctx.db, id).await?;
    let active = found.into_active_model().activated(&ctx.db).await?;
    format::json(configuration(&active))
}

#[debug_handler]
pub async fn deactivate(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let found = Model::find(&ctx.db, id).await?;
    let idle = found.into_active_model().deactivated(&ctx.db).await?;
    format::json(configuration(&idle))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/configurations")
        .add("/", get(list))
        .add("/", post(create))
        .add("/{id}", get(show))
        .add("/{id}", patch(update))
        .add("/{id}", delete(remove))
        .add("/{id}/activate", post(activate))
        .add("/{id}/deactivate", post(deactivate))
}
