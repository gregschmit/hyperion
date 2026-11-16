#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    models::{
        categories,
        watchlist_items::{self, Params},
    },
    views,
};

/// The watchlist's page, with `form` in its form for a new item, and what's wrong, if
/// something is.
pub async fn page(
    v: &TeraView,
    ctx: &AppContext,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let items: Vec<_> = watchlist_items::Model::list(&ctx.db, None)
        .await?
        .iter()
        .map(views::watchlist_item)
        .collect();
    let categories: Vec<_> = categories::Model::list(&ctx.db)
        .await?
        .iter()
        .map(views::category)
        .collect();
    let data = json!({
        "items": items,
        "categories": categories,
        "form": form,
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, "watchlist/index.html", "watchlist", data).await
}

/// The watchlist: the issues and PRs/MRs that you follow.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    page(&v, &ctx, &Params::default(), None).await
}

#[debug_handler]
pub async fn add(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(params): Form<Params>,
) -> Result<Response> {
    match watchlist_items::Model::create(&ctx.db, &params).await? {
        Ok(_) => format::redirect("/watchlist"),
        Err(problem) => page(&v, &ctx, &params, Some(problem)).await,
    }
}

#[debug_handler]
pub async fn update(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let (item, _) = watchlist_items::Model::find(&ctx.db, id).await?;
    // A row's form sends no URL: an item keeps its own.
    let params = Params {
        url: item.url.clone(),
        ..params
    };
    match item.revised(&ctx.db, &params).await? {
        Ok(_) => format::redirect("/watchlist"),
        Err(problem) => page(&v, &ctx, &Params::default(), Some(problem)).await,
    }
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    watchlist_items::Model::remove(&ctx.db, id).await?;
    format::redirect("/watchlist")
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/watchlist")
        .add("/", get(index))
        .add("/", post(add))
        .add("/{id}", post(update))
        .add("/{id}/remove", post(remove))
}
