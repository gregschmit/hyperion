#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::{
    models::{
        categories, configurations, runs,
        schedules::{self, Operation, Params},
    },
    prompts,
    settings::Settings,
    views,
};

/// When each operation runs, where, with which command, and with which prompt.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let rows: Vec<_> = schedules::Model::list(&ctx.db)
        .await?
        .iter()
        .map(views::schedule)
        .collect();
    let data = json!({
        "schedules": rows,
        "configured": configurations::Model::active(&ctx.db).await?.is_some(),
    });
    views::page(&v, &ctx, "schedules/index.html", "schedules", data).await
}

/// The form for a new schedule (no `id`) or a saved one, with the prompt compiled as the
/// next run would get it.
async fn form(
    v: &TeraView,
    ctx: &AppContext,
    id: Option<i64>,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let settings = Settings::from_context(ctx)?;
    // The page fills in an operation's command when you choose the operation.
    let operations: Vec<Value> = Operation::ALL
        .into_iter()
        .map(|operation| {
            json!({
                "name": operation.as_str(),
                "command": operation.default_command(&settings.agent),
            })
        })
        .collect();
    let categories = categories::Model::list(&ctx.db).await?;
    let variables: Vec<Value> = prompts::VARIABLES
        .into_iter()
        .map(|(name, meaning)| json!({ "name": name, "meaning": meaning }))
        .collect();
    // With the watchlist as it is now, and the category that the form names, if it's one.
    let category = categories.iter().find(|it| it.name == form.category.trim());
    let compiled = prompts::Context::load(&ctx.db, category, prompts::SAMPLE_ARTIFACTS_DIR)
        .await?
        .compile(&form.prompt);
    let data = json!({
        "is_new": id.is_none(),
        "id": id.unwrap_or_default(),
        "form": form,
        "operations": operations,
        "categories": categories.iter().map(views::category).collect::<Vec<_>>(),
        "variables": variables,
        "compiles": compiled.is_ok(),
        "compiled": compiled.unwrap_or_else(|problem| problem),
        "configured": configurations::Model::active(&ctx.db).await?.is_some(),
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, "schedules/form.html", "schedules", data).await
}

#[debug_handler]
pub async fn new(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let agent = Settings::from_context(&ctx)?.agent;
    form(&v, &ctx, None, &Params::starter(&agent), None).await
}

#[debug_handler]
pub async fn create(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(params): Form<Params>,
) -> Result<Response> {
    match schedules::Model::create(&ctx.db, &params).await? {
        Ok(_) => format::redirect("/schedules"),
        Err(problem) => form(&v, &ctx, None, &params, Some(problem)).await,
    }
}

#[debug_handler]
pub async fn edit(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
) -> Result<Response> {
    let schedule = schedules::Model::find(&ctx.db, id).await?;
    let params = Params::changed(&schedule, schedules::Changes::default());
    form(&v, &ctx, Some(id), &params, None).await
}

#[debug_handler]
pub async fn update(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let (schedule, _) = schedules::Model::find(&ctx.db, id).await?;
    match schedule.revised(&ctx.db, &params).await? {
        Ok(_) => format::redirect(&format!("/schedules/{id}")),
        Err(problem) => form(&v, &ctx, Some(id), &params, Some(problem)).await,
    }
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    schedules::Model::remove(&ctx.db, id).await?;
    format::redirect("/schedules")
}

/// Queue the schedule's run now, to try its command and prompt. The next tick starts it.
#[debug_handler]
pub async fn run(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let (schedule, _) = schedules::Model::find(&ctx.db, id).await?;
    runs::Model::queue(&ctx.db, &schedule).await?;
    format::redirect("/runs")
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/schedules")
        .add("/", get(index))
        .add("/", post(create))
        .add("/new", get(new))
        .add("/{id}", get(edit))
        .add("/{id}", post(update))
        .add("/{id}/remove", post(remove))
        .add("/{id}/run", post(run))
}
