#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    models::{
        configurations::{self, Operation},
        runs,
        schedules::{self, Params},
    },
    views,
};

async fn page(
    v: &TeraView,
    ctx: &AppContext,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let rows: Vec<_> = schedules::Model::list(&ctx.db)
        .await?
        .iter()
        .map(views::schedule)
        .collect();
    let data = json!({
        "schedules": rows,
        "operations": Operation::ALL.map(Operation::as_str),
        "configured": configurations::Model::active(&ctx.db).await?.is_some(),
        "form": form,
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, "schedules/index.html", "schedules", data).await
}

/// When each operation runs, and where.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let form = Params {
        operation: Operation::Triage.as_str().to_string(),
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
    schedules::Model::create(&ctx.db, &params).await?;
    format::redirect("/schedules")
}

#[debug_handler]
pub async fn update(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    let schedule = schedules::Model::find(&ctx.db, id).await?;
    if let Some(problem) = params.problem() {
        return page(&v, &ctx, &params, Some(problem)).await;
    }
    schedule
        .into_active_model()
        .revised(&ctx.db, &params)
        .await?;
    format::redirect("/schedules")
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    schedules::Model::find(&ctx.db, id)
        .await?
        .delete(&ctx.db)
        .await?;
    format::redirect("/schedules")
}

/// Queue the schedule's run now, to test a prompt. The next tick starts it.
#[debug_handler]
pub async fn run(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let schedule = schedules::Model::find(&ctx.db, id).await?;
    let Some(operation) = Operation::parse(&schedule.operation) else {
        return bad_request("the schedule has no such operation");
    };
    runs::Model::queue(&ctx.db, operation, &schedule.repo).await?;
    format::redirect("/runs")
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/schedules")
        .add("/", get(index))
        .add("/", post(add))
        .add("/{id}", post(update))
        .add("/{id}/remove", post(remove))
        .add("/{id}/run", post(run))
}
