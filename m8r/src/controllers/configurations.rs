#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::{
    models::{
        configurations::{self, Operation, Params, SAMPLE_ARTIFACTS_DIR},
        items,
    },
    views,
};

/// The form for a new configuration (no `id`) or a saved one, with each prompt compiled as
/// the next run would get it.
async fn form(
    v: &TeraView,
    ctx: &AppContext,
    id: Option<i64>,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let items = items::Model::list(&ctx.db).await?;
    let compiled: Vec<Value> = Operation::ALL
        .into_iter()
        .map(|operation| {
            let prompt = form.compile(operation, &items, SAMPLE_ARTIFACTS_DIR);
            json!({
                "operation": operation.as_str(),
                "ok": prompt.is_ok(),
                "blank": prompt.as_ref().is_ok_and(|prompt| prompt.trim().is_empty()),
                "text": prompt.unwrap_or_else(|problem| problem),
            })
        })
        .collect();
    let data = json!({
        "is_new": id.is_none(),
        "id": id.unwrap_or_default(),
        "form": form,
        "compiled": compiled,
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, "configurations/form.html", "configurations", data).await
}

/// The sets of prompts. One is active, or none, and then nothing runs.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let rows: Vec<Value> = configurations::Model::list(&ctx.db)
        .await?
        .iter()
        .map(|configuration| {
            json!({
                "id": configuration.id,
                "name": configuration.name,
                "active": configuration.active,
                "updated_at": views::time(&configuration.updated_at),
            })
        })
        .collect();
    let data = json!({ "configurations": rows });
    views::page(
        &v,
        &ctx,
        "configurations/index.html",
        "configurations",
        data,
    )
    .await
}

#[debug_handler]
pub async fn new(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    form(&v, &ctx, None, &Params::starter(), None).await
}

#[debug_handler]
pub async fn create(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let items = items::Model::list(&ctx.db).await?;
    if let Some(problem) = params.problem(&items) {
        return form(&v, &ctx, None, &params, Some(problem)).await;
    }
    let configuration = configurations::Model::create(&ctx.db, &params).await?;
    format::redirect(&format!("/configurations/{}", configuration.id))
}

#[debug_handler]
pub async fn edit(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
) -> Result<Response> {
    let configuration = configurations::Model::find(&ctx.db, id).await?;
    form(&v, &ctx, Some(id), &Params::from(&configuration), None).await
}

#[debug_handler]
pub async fn update(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let configuration = configurations::Model::find(&ctx.db, id).await?;
    let items = items::Model::list(&ctx.db).await?;
    if let Some(problem) = params.problem(&items) {
        return form(&v, &ctx, Some(id), &params, Some(problem)).await;
    }
    configuration
        .into_active_model()
        .revised(&ctx.db, &params)
        .await?;
    format::redirect(&format!("/configurations/{id}"))
}

#[debug_handler]
pub async fn activate(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    configurations::Model::find(&ctx.db, id)
        .await?
        .into_active_model()
        .activated(&ctx.db)
        .await?;
    format::redirect("/configurations")
}

#[debug_handler]
pub async fn deactivate(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    configurations::Model::find(&ctx.db, id)
        .await?
        .into_active_model()
        .deactivated(&ctx.db)
        .await?;
    format::redirect("/configurations")
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    configurations::Model::find(&ctx.db, id)
        .await?
        .delete(&ctx.db)
        .await?;
    format::redirect("/configurations")
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/configurations")
        .add("/", get(index))
        .add("/", post(create))
        .add("/new", get(new))
        .add("/{id}", get(edit))
        .add("/{id}", post(update))
        .add("/{id}/activate", post(activate))
        .add("/{id}/deactivate", post(deactivate))
        .add("/{id}/remove", post(remove))
}
