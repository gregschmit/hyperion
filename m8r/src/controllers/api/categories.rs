#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;

use super::saved;
use crate::{
    models::categories::{Model, Params},
    views::api::category,
};

#[debug_handler]
pub async fn list(State(ctx): State<AppContext>) -> Result<Response> {
    let categories = Model::list(&ctx.db).await?;
    format::json(categories.iter().map(category).collect::<Vec<_>>())
}

#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    format::json(category(&Model::find(&ctx.db, id).await?))
}

#[debug_handler]
pub async fn create(State(ctx): State<AppContext>, Json(params): Json<Params>) -> Result<Response> {
    saved(Model::create(&ctx.db, &params).await?, category)
}

/// Rename it.
#[debug_handler]
pub async fn update(
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Json(params): Json<Params>,
) -> Result<Response> {
    let found = Model::find(&ctx.db, id).await?;
    saved(found.renamed(&ctx.db, &params).await?, category)
}

/// Remove it, unless a watchlist item or a schedule uses it.
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
        .prefix("/api/categories")
        .add("/", get(list))
        .add("/", post(create))
        .add("/{id}", get(show))
        .add("/{id}", patch(update))
        .add("/{id}", delete(remove))
}
