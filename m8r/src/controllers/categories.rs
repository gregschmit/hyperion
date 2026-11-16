#![allow(clippy::missing_errors_doc)]
//! The categories are on the watchlist's page, so that's where these go back to.

use loco_rs::prelude::*;

use super::watchlist;
use crate::models::{
    categories::{Model, Params},
    watchlist_items, Checked,
};

/// Back to the watchlist, which says what's wrong if something is.
async fn back<T>(v: &TeraView, ctx: &AppContext, checked: Checked<T>) -> Result<Response> {
    match checked {
        Ok(_) => format::redirect("/watchlist"),
        Err(problem) => {
            let form = watchlist_items::Params::default();
            watchlist::page(v, ctx, &form, Some(problem)).await
        }
    }
}

#[debug_handler]
pub async fn add(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(params): Form<Params>,
) -> Result<Response> {
    back(&v, &ctx, Model::create(&ctx.db, &params).await?).await
}

#[debug_handler]
pub async fn rename(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let category = Model::find(&ctx.db, id).await?;
    back(&v, &ctx, category.renamed(&ctx.db, &params).await?).await
}

#[debug_handler]
pub async fn remove(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
) -> Result<Response> {
    let category = Model::find(&ctx.db, id).await?;
    back(&v, &ctx, category.removed(&ctx.db).await?).await
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/categories")
        .add("/", post(add))
        .add("/{id}", post(rename))
        .add("/{id}/remove", post(remove))
}
