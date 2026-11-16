#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::{
    models::configurations::{self, Params},
    prompts, views,
};

/// The form for a new configuration (no `id`) or a saved one, with the system message
/// compiled as the next run would get it.
async fn form(
    v: &TeraView,
    ctx: &AppContext,
    id: Option<i64>,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let compiled = form.compiled(&ctx.db).await?;
    // The system message can use what a prompt can, but for itself.
    let variables: Vec<Value> = prompts::VARIABLES[1..]
        .iter()
        .map(|(name, meaning)| json!({ "name": name, "meaning": meaning }))
        .collect();
    let data = json!({
        "is_new": id.is_none(),
        "id": id.unwrap_or_default(),
        "form": form,
        "variables": variables,
        "compiles": compiled.is_ok(),
        "compiled": compiled.unwrap_or_else(|problem| problem),
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, "configurations/form.html", "configurations", data).await
}

/// The system messages. One configuration is active, or none, and then nothing runs.
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
    match configurations::Model::create(&ctx.db, &params).await? {
        Ok(configuration) => format::redirect(&format!("/configurations/{}", configuration.id)),
        Err(problem) => form(&v, &ctx, None, &params, Some(problem)).await,
    }
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
    match configuration.revised(&ctx.db, &params).await? {
        Ok(_) => format::redirect(&format!("/configurations/{id}")),
        Err(problem) => form(&v, &ctx, Some(id), &params, Some(problem)).await,
    }
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
