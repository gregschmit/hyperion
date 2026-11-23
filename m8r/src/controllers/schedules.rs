#![allow(clippy::missing_errors_doc)]
use axum::Extension;
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    initializers::access::Caller,
    models::{
        repos, runs,
        schedules::{self, Items, Params},
    },
    prompts,
    settings::Settings,
    views,
};

/// What a schedule's form sends. A repo's checkbox sends `repos` once, so a form can send it
/// several times, and its number is text. The box for `enabled` sends it only when it's
/// checked.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Sent {
    name: String,
    category: String,
    max_items: String,
    repos: Vec<String>,
    when: String,
    enabled: bool,
    command: String,
    prompt: String,
}

impl Sent {
    /// What was sent, as a schedule takes it, and what's wrong with the number, if it isn't
    /// one.
    fn params(self) -> (Params, Option<String>) {
        let max_items = match self.max_items.trim() {
            "" => Ok(0),
            text => text.parse(),
        };
        let problem = max_items
            .is_err()
            .then(|| "Max items is a whole number: 0 for all of them.".to_string());
        let params = Params {
            name: self.name,
            category: self.category,
            max_items: max_items.unwrap_or_default(),
            repos: self.repos.join("\n").as_str().into(),
            when: self.when.as_str().into(),
            enabled: self.enabled,
            command: self.command,
            prompt: self.prompt,
        };
        (params, problem)
    }
}

/// When each schedule runs, where, with which command, and with which prompt.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
) -> Result<Response> {
    let rows: Vec<_> = schedules::Model::list(&ctx.db)
        .await?
        .iter()
        .map(views::schedule)
        .collect();
    let data = json!({ "schedules": rows });
    views::page(&v, &ctx, &caller, "schedules/index.html", "schedules", data).await
}

/// The form for a new schedule (no `id`) or a saved one, with the prompt compiled as the
/// next run would get it in the first of its repos.
async fn form(
    v: &TeraView,
    ctx: &AppContext,
    caller: &Caller,
    id: Option<i64>,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let known = repos::Model::list(&ctx.db).await?;
    let chosen: Vec<&repos::Model> = form
        .repos
        .iter()
        .filter_map(|path| known.iter().find(|repo| repo.is_at(path)))
        .collect();
    let repos: Vec<Value> = known
        .iter()
        .map(|repo| {
            let checked = chosen.iter().any(|it| it.id == repo.id);
            json!({ "path": repo.path, "checked": checked })
        })
        .collect();
    let choices: Vec<Value> = Items::CHOICES
        .into_iter()
        .map(|items| {
            let name = match items {
                Items::None => "no items",
                _ => items.as_str(),
            };
            json!({ "value": items.as_str(), "name": name })
        })
        .collect();
    let variables: Vec<Value> = prompts::VARIABLES
        .into_iter()
        .map(|(name, meaning)| json!({ "name": name, "meaning": meaning }))
        .collect();
    // With the watchlist as it is now, in the first repo that the form names.
    let shown = chosen.first().copied();
    let compiled = form.context(&ctx.db, shown).await?.compile(&form.prompt);
    let data = json!({
        "is_new": id.is_none(),
        "id": id.unwrap_or_default(),
        "form": form,
        "repos": repos,
        "choices": choices,
        "variables": variables,
        "compiled_in": shown.map(|repo| repo.path.as_str()).unwrap_or_default(),
        "compiles": compiled.is_ok(),
        "compiled": compiled.unwrap_or_else(|problem| problem),
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, caller, "schedules/form.html", "schedules", data).await
}

#[debug_handler]
pub async fn new(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
) -> Result<Response> {
    let agent = Settings::from_context(&ctx)?.agent;
    form(&v, &ctx, &caller, None, &Params::starter(&agent), None).await
}

#[debug_handler]
pub async fn create(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
    axum_extra::extract::Form(sent): axum_extra::extract::Form<Sent>,
) -> Result<Response> {
    let (params, problem) = sent.params();
    if problem.is_some() {
        return form(&v, &ctx, &caller, None, &params, problem).await;
    }
    match schedules::Model::create(&ctx.db, &params).await? {
        Ok(_) => format::redirect("/schedules"),
        Err(problem) => form(&v, &ctx, &caller, None, &params, Some(problem)).await,
    }
}

#[debug_handler]
pub async fn edit(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
) -> Result<Response> {
    let schedule = schedules::Model::find(&ctx.db, id).await?;
    let params = Params::changed(&schedule, schedules::Changes::default());
    form(&v, &ctx, &caller, Some(id), &params, None).await
}

#[debug_handler]
pub async fn update(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
    axum_extra::extract::Form(sent): axum_extra::extract::Form<Sent>,
) -> Result<Response> {
    let (schedule, _) = schedules::Model::find(&ctx.db, id).await?;
    let (params, problem) = sent.params();
    if problem.is_some() {
        return form(&v, &ctx, &caller, Some(id), &params, problem).await;
    }
    match schedule.revised(&ctx.db, &params).await? {
        Ok(_) => format::redirect(&format!("/schedules/{id}")),
        Err(problem) => form(&v, &ctx, &caller, Some(id), &params, Some(problem)).await,
    }
}

#[debug_handler]
pub async fn remove(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    schedules::Model::remove(&ctx.db, id).await?;
    format::redirect("/schedules")
}

/// Turn the schedule on: it runs at its times again.
#[debug_handler]
pub async fn enable(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    schedules::Model::switch(&ctx.db, id, true).await?;
    format::redirect("/schedules")
}

/// Turn the schedule off: it starts nothing at its times, and its runs that wait are dropped.
#[debug_handler]
pub async fn disable(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    schedules::Model::switch(&ctx.db, id, false).await?;
    format::redirect("/schedules")
}

/// Queue the schedule's runs now, one for each repo, to try its command and prompt. The next
/// tick starts them, whether the schedule is on or off.
#[debug_handler]
pub async fn run(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let (schedule, repos) = schedules::Model::find(&ctx.db, id).await?;
    runs::Model::queue(&ctx.db, &schedule, &repos).await?;
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
        .add("/{id}/enable", post(enable))
        .add("/{id}/disable", post(disable))
        .add("/{id}/remove", post(remove))
        .add("/{id}/run", post(run))
}
