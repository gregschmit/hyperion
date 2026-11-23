#![allow(clippy::missing_errors_doc)]
use axum::Extension;
use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    initializers::access::Caller,
    models::{
        repos::{Model, Params},
        Checked,
    },
    settings::Settings,
    views,
};

/// The repos' page, with `form` in its form for a new repo, and what's wrong, if something
/// is.
async fn page(
    v: &TeraView,
    ctx: &AppContext,
    caller: &Caller,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let mut repos = Vec::new();
    for repo in Model::list(&ctx.db).await? {
        let uses = repo.uses(&ctx.db).await?;
        repos.push(views::repo(&repo, uses));
    }
    let data = json!({
        "repos": repos,
        "form": form,
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, caller, "repos/index.html", "repos", data).await
}

/// Back to the repos, which say what's wrong if something is.
async fn back<T>(
    v: &TeraView,
    ctx: &AppContext,
    caller: &Caller,
    checked: Checked<T>,
) -> Result<Response> {
    match checked {
        Ok(_) => format::redirect("/repos"),
        Err(problem) => page(v, ctx, caller, &Params::default(), Some(problem)).await,
    }
}

/// The repos that m8r works in: each a clone, and the project on its forge.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
) -> Result<Response> {
    page(&v, &ctx, &caller, &Params::default(), None).await
}

#[debug_handler]
pub async fn add(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let shell = Settings::from_context(&ctx)?.shell;
    match Model::create(&ctx.db, &shell, &params).await? {
        Ok(_) => format::redirect("/repos"),
        Err(problem) => page(&v, &ctx, &caller, &params, Some(problem)).await,
    }
}

#[debug_handler]
pub async fn update(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let shell = Settings::from_context(&ctx)?.shell;
    let repo = Model::find(&ctx.db, id).await?;
    back(
        &v,
        &ctx,
        &caller,
        repo.revised(&ctx.db, &shell, &params).await?,
    )
    .await
}

#[debug_handler]
pub async fn remove(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
) -> Result<Response> {
    let repo = Model::find(&ctx.db, id).await?;
    back(&v, &ctx, &caller, repo.removed(&ctx.db).await?).await
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/repos")
        .add("/", get(index))
        .add("/", post(add))
        .add("/{id}", post(update))
        .add("/{id}/remove", post(remove))
}
