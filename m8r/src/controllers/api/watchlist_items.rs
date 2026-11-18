#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde::Deserialize;

use super::saved;
use crate::{
    models::watchlist_items::{self, Category, Changes, Model, Params},
    views::api::watchlist_item,
};

#[derive(Debug, Deserialize)]
pub struct Filter {
    /// A category's name: only its items.
    pub category: Option<String>,
    /// A repo's id: only its items.
    pub repo_id: Option<i64>,
}

#[debug_handler]
pub async fn list(State(ctx): State<AppContext>, Query(filter): Query<Filter>) -> Result<Response> {
    let category = match Category::named(filter.category.as_deref()) {
        Ok(category) => category,
        Err(problem) => return bad_request(problem),
    };
    let filter = watchlist_items::Filter {
        repo_id: filter.repo_id,
        category,
    };
    let items = Model::list(&ctx.db, filter).await?;
    format::json(items.iter().map(watchlist_item).collect::<Vec<_>>())
}

#[debug_handler]
pub async fn show(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    format::json(watchlist_item(&Model::find(&ctx.db, id).await?))
}

/// Add an item, or change the one that has this URL.
#[debug_handler]
pub async fn create(State(ctx): State<AppContext>, Json(params): Json<Params>) -> Result<Response> {
    saved(Model::create(&ctx.db, &params).await?, watchlist_item)
}

#[debug_handler]
pub async fn update(
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Json(changes): Json<Changes>,
) -> Result<Response> {
    saved(Model::change(&ctx.db, id, changes).await?, watchlist_item)
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    Model::remove(&ctx.db, id).await?;
    format::empty_json()
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/watchlist_items")
        .add("/", get(list))
        .add("/", post(create))
        .add("/{id}", get(show))
        .add("/{id}", patch(update))
        .add("/{id}", delete(remove))
}
