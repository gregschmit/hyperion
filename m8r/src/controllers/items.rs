#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    models::items::{self, Params},
    views,
};

async fn page(
    v: &TeraView,
    ctx: &AppContext,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let rows: Vec<_> = items::Model::list(&ctx.db)
        .await?
        .iter()
        .map(views::item)
        .collect();
    let data = json!({
        "items": rows,
        "form": form,
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, "items/index.html", "items", data).await
}

/// The issues and PRs/MRs that you care about.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let form = Params {
        priority: items::Priority::Medium.as_str().to_string(),
        ..Params::default()
    };
    page(&v, &ctx, &form, None).await
}

#[debug_handler]
pub async fn add(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(params): Form<Params>,
) -> Result<Response> {
    if let Some(problem) = params.problem() {
        return page(&v, &ctx, &params, Some(problem)).await;
    }
    items::Model::create(&ctx.db, &params).await?;
    format::redirect("/items")
}

#[debug_handler]
pub async fn update(
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let item = items::Model::find(&ctx.db, id).await?;
    if items::Priority::parse(&params.priority).is_none() {
        return bad_request("the priority is medium or high");
    }
    item.into_active_model().revised(&ctx.db, &params).await?;
    format::redirect("/items")
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    items::Model::find(&ctx.db, id)
        .await?
        .delete(&ctx.db)
        .await?;
    format::redirect("/items")
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/items")
        .add("/", get(index))
        .add("/", post(add))
        .add("/{id}", post(update))
        .add("/{id}/remove", post(remove))
}
