//! The MCP endpoint, `/mcp`: the API as tools, for an agent that isn't one of m8r's runs, such
//! as Claude Duet.
//!
//! It speaks streamable HTTP with no sessions: each POST stands alone, so a restart loses
//! nothing. The access guard lets a request in with a token (see `initializers::access`). A
//! caller sees and calls only the tools of the resources that its token covers.

use std::sync::Arc;

use axum::{http::request::Parts, routing::any_service};
use loco_rs::prelude::*;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
        Tool, ToolAnnotations,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};

use super::api::runs::RUNS;
use crate::{
    initializers::access::Caller,
    models::{artifacts, categories, configurations, runs, schedules, watchlist_items, Checked},
    settings::Settings,
    views::api,
};

/// What every client reads first.
const INSTRUCTIONS: &str = "\
m8r runs the developer's coding agents on a schedule, and keeps the developer's watchlist: the \
issues and PRs/MRs that they follow. A watchlist item is a URL on GitHub or GitLab, with a \
category, a priority (medium: the developer cares about it; high: it's urgent), a note that \
says why, and for a PR/MR, whether someone asked the developer to review it \
(review_requested). The agents get the watchlist in their prompts, so keep it true: add what \
the developer must follow, turn review_requested on when someone asks them for a review, and \
remove what's done. m8r removes an item by itself when its issue or PR/MR closes or merges.";

/// What a tool does to m8r, which the client's approval step goes by.
#[derive(Clone, Copy)]
enum Effect {
    Reads,
    /// Makes something new each time.
    Adds,
    /// Sets something. Twice is the same as once.
    Sets,
    Removes,
}

struct Spec {
    name: &'static str,
    title: &'static str,
    /// The resource that a token must cover (see `tokens::RESOURCES`).
    resource: &'static str,
    effect: Effect,
    description: &'static str,
}

const fn spec(
    name: &'static str,
    title: &'static str,
    resource: &'static str,
    effect: Effect,
    description: &'static str,
) -> Spec {
    Spec {
        name,
        title,
        resource,
        effect,
        description,
    }
}

const TOOLS: [Spec; 21] = [
    spec(
        "list_watchlist_items",
        "List the watchlist",
        "watchlist_items",
        Effect::Reads,
        "List the watchlist: the issues and PRs/MRs that the developer follows. Each item has \
         an id, a url, a category, a priority, a note, review_requested, and the title that \
         the forge gives it.",
    ),
    spec(
        "add_watchlist_item",
        "Add a watchlist item",
        "watchlist_items",
        Effect::Sets,
        "Put an issue or a PR/MR on the watchlist, by its URL on GitHub or GitLab. If the URL \
         is already there, this changes that item.",
    ),
    spec(
        "update_watchlist_item",
        "Change a watchlist item",
        "watchlist_items",
        Effect::Sets,
        "Change a watchlist item: its category, priority, note or review_requested. What you \
         leave out stays as it is.",
    ),
    spec(
        "remove_watchlist_item",
        "Remove a watchlist item",
        "watchlist_items",
        Effect::Removes,
        "Take an item off the watchlist, when the developer no longer follows it.",
    ),
    spec(
        "list_categories",
        "List the categories",
        "categories",
        Effect::Reads,
        "List the categories that watchlist items are in.",
    ),
    spec(
        "add_category",
        "Add a category",
        "categories",
        Effect::Adds,
        "Add a category for watchlist items.",
    ),
    spec(
        "rename_category",
        "Rename a category",
        "categories",
        Effect::Sets,
        "Give a category another name. Its items and schedules keep it.",
    ),
    spec(
        "remove_category",
        "Remove a category",
        "categories",
        Effect::Removes,
        "Remove a category. m8r refuses while a watchlist item or a schedule uses it.",
    ),
    spec(
        "list_schedules",
        "List the schedules",
        "schedules",
        Effect::Reads,
        "List the schedules: when each one runs, in which clones, with which command and which \
         prompt, and its next time.",
    ),
    spec(
        "add_schedule",
        "Add a schedule",
        "schedules",
        Effect::Adds,
        "Add a schedule: m8r then runs its command in each of its repos at its times, with its \
         prompt on stdin.",
    ),
    spec(
        "update_schedule",
        "Change a schedule",
        "schedules",
        Effect::Sets,
        "Change a schedule. What you leave out stays as it is.",
    ),
    spec(
        "remove_schedule",
        "Remove a schedule",
        "schedules",
        Effect::Removes,
        "Remove a schedule. Its past runs stay in the history.",
    ),
    spec(
        "run_schedule",
        "Run a schedule now",
        "schedules",
        Effect::Adds,
        "Queue a schedule's runs now: one for each of its repos, which start together. The \
         schedules take turns, so they may wait. The answer is the runs.",
    ),
    spec(
        "list_configurations",
        "List the configurations",
        "configurations",
        Effect::Reads,
        "List the configurations. Each holds a prompt, the developer's part of every \
         schedule's prompt, and the runs get the active one's.",
    ),
    spec(
        "add_configuration",
        "Add a configuration",
        "configurations",
        Effect::Adds,
        "Add a configuration. It isn't active until you activate it.",
    ),
    spec(
        "update_configuration",
        "Change a configuration",
        "configurations",
        Effect::Sets,
        "Change a configuration's name or prompt. What you leave out stays as it is.",
    ),
    spec(
        "remove_configuration",
        "Remove a configuration",
        "configurations",
        Effect::Removes,
        "Remove a configuration. If it's the active one, nothing runs after that.",
    ),
    spec(
        "activate_configuration",
        "Activate a configuration",
        "configurations",
        Effect::Sets,
        "Make a configuration the active one. The one that was active stops being it.",
    ),
    spec(
        "deactivate_configuration",
        "Deactivate a configuration",
        "configurations",
        Effect::Sets,
        "Make a configuration inactive. With none active, nothing runs.",
    ),
    spec(
        "list_runs",
        "List the runs",
        "runs",
        Effect::Reads,
        "List the latest runs, newest first: the queue and the history, each with its status \
         and the artifacts that its agent reported.",
    ),
    spec(
        "get_run",
        "Get a run",
        "runs",
        Effect::Reads,
        "Get one run, with the prompt it got and what its command printed.",
    ),
];

impl Spec {
    /// The tool as a client sees it. `categories` are the ones there are now, which the
    /// schema offers.
    fn tool(&self, categories: &[String]) -> Tool {
        let hints = ToolAnnotations::with_title(self.title).open_world(false);
        let hints = match self.effect {
            Effect::Reads => hints.read_only(true),
            Effect::Adds => hints.read_only(false).destructive(false).idempotent(false),
            Effect::Sets => hints.read_only(false).destructive(false).idempotent(true),
            Effect::Removes => hints.read_only(false).destructive(true).idempotent(true),
        };
        Tool::new(self.name, self.description, self.input(categories))
            .with_title(self.title)
            .with_annotations(hints)
    }

    /// The JSON schema of the tool's arguments.
    fn input(&self, categories: &[String]) -> JsonObject {
        let id = |of: &str| {
            let description = format!("The id of the {of}, as its list shows it.");
            json!({ "type": "integer", "description": description })
        };
        let text = |description: &str| json!({ "type": "string", "description": description });
        let mut category = text("The name of a category.");
        if !categories.is_empty() {
            category["enum"] = json!(categories);
        }
        let priority = json!({
            "type": "string",
            "enum": ["medium", "high"],
            "description": "medium: the developer cares about it. high: it's urgent.",
        });
        let note = text("Why the developer follows it, in a few words.");
        let review = json!({
            "type": "boolean",
            "description": "Someone asked the developer to review this PR/MR. An issue has none.",
        });
        let operation = json!({
            "type": "string",
            "enum": ["triage", "review", "develop", "other"],
            "description": "What the schedule is for.",
        });
        let run_category = {
            let mut category = category.clone();
            category["description"] =
                json!("The category whose watchlist items the prompt gets. Blank: no items.");
            if let Some(names) = category.get_mut("enum").and_then(Value::as_array_mut) {
                names.push(json!(""));
            }
            category
        };
        let texts = |description: &str| json!({ "type": "array", "items": { "type": "string" }, "description": description });
        let repos = texts(
            "The clones that the runs start in, one run in each: paths from the home of m8r's \
             user, such as ~/git/group/project.",
        );
        let hron = texts(
            "When it runs, in words: \"every weekday at 06:00\", \"every 12 hours\", \"every \
             day at 09:00, 17:00\". A five-field cron expression works too. With several, it \
             runs at each one's times.",
        );
        let command = text("The exact shell line that a run starts, such as \"claude -p\".");
        let prompt = text(
            "What the command gets on stdin: a template that can hold {{ m8r_prompt }}, \
             {{ config_prompt }} and {{ watchlist }}.",
        );
        let name = |of: &str| text(&format!("The {of}'s name."));
        let config_prompt = text(
            "The developer's part of every prompt: a template that a schedule's prompt puts in \
             with {{ config_prompt }}.",
        );

        let (properties, required): (Value, &[&str]) = match self.name {
            "list_watchlist_items" => (json!({ "category": category }), &[]),
            "add_watchlist_item" => (
                json!({
                    "url": text("The URL of the issue or PR/MR."),
                    "category": category,
                    "priority": priority,
                    "note": note,
                    "review_requested": review,
                }),
                &["url", "category"],
            ),
            "update_watchlist_item" => (
                json!({
                    "id": id("item"),
                    "category": category,
                    "priority": priority,
                    "note": note,
                    "review_requested": review,
                }),
                &["id"],
            ),
            "remove_watchlist_item" => (json!({ "id": id("item") }), &["id"]),
            "add_category" => (json!({ "name": name("category") }), &["name"]),
            "rename_category" => (
                json!({ "id": id("category"), "name": name("category") }),
                &["id", "name"],
            ),
            "remove_category" => (json!({ "id": id("category") }), &["id"]),
            "add_schedule" => (
                json!({
                    "operation": operation,
                    "category": run_category,
                    "repos": repos,
                    "hron": hron,
                    "command": command,
                    "prompt": prompt,
                }),
                &["operation", "repos", "hron", "command"],
            ),
            "update_schedule" => (
                json!({
                    "id": id("schedule"),
                    "operation": operation,
                    "category": run_category,
                    "repos": repos,
                    "hron": hron,
                    "command": command,
                    "prompt": prompt,
                }),
                &["id"],
            ),
            "remove_schedule" | "run_schedule" => (json!({ "id": id("schedule") }), &["id"]),
            "add_configuration" => (
                json!({ "name": name("configuration"), "prompt": config_prompt }),
                &["name"],
            ),
            "update_configuration" => (
                json!({
                    "id": id("configuration"),
                    "name": name("configuration"),
                    "prompt": config_prompt,
                }),
                &["id"],
            ),
            "remove_configuration" | "activate_configuration" | "deactivate_configuration" => {
                (json!({ "id": id("configuration") }), &["id"])
            }
            "list_runs" => (
                json!({ "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": RUNS,
                    "description": format!("How many runs, {RUNS} at most."),
                } }),
                &[],
            ),
            "get_run" => (json!({ "id": id("run") }), &["id"]),
            _ => (json!({}), &[]),
        };
        let schema = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        });
        match schema {
            Value::Object(schema) => schema,
            _ => JsonObject::new(),
        }
    }
}

/// The arguments of a tool that acts on one thing, by its id.
#[derive(Debug, Deserialize)]
struct Target<T> {
    id: i64,
    #[serde(flatten)]
    rest: T,
}

#[derive(Debug, Default, Deserialize)]
struct Nothing {}

#[derive(Debug, Default, Deserialize)]
struct ItemFilter {
    category: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RunFilter {
    limit: Option<u64>,
}

/// Why a tool didn't do what it was asked.
enum Refusal {
    /// The caller's to fix: the model reads it, and can try again.
    Problem(String),
    /// m8r's own failure.
    Failure(String),
}

impl From<ModelError> for Refusal {
    fn from(err: ModelError) -> Self {
        match err {
            ModelError::EntityNotFound => Self::Problem("m8r has nothing with that id.".into()),
            other => Self::Failure(other.to_string()),
        }
    }
}

impl From<Error> for Refusal {
    fn from(err: Error) -> Self {
        Self::Failure(err.to_string())
    }
}

fn checked<T>(checked: Checked<T>) -> std::result::Result<T, Refusal> {
    checked.map_err(Refusal::Problem)
}

/// Read a tool's arguments. What can't be read is the caller's to fix.
fn parse<T: DeserializeOwned>(arguments: Option<JsonObject>) -> std::result::Result<T, Refusal> {
    serde_json::from_value(Value::Object(arguments.unwrap_or_default()))
        .map_err(|err| Refusal::Problem(format!("m8r can't read the arguments: {err}.")))
}

/// The handler of one request: rmcp makes one for each POST.
struct Server {
    ctx: AppContext,
}

impl Server {
    /// Do what the tool `name` does, and give back what a client reads.
    async fn call(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<Value, Refusal> {
        let db = &self.ctx.db;
        Ok(match name {
            "list_watchlist_items" => {
                let filter: ItemFilter = parse(arguments)?;
                let category = match filter.category.filter(|name| !name.trim().is_empty()) {
                    Some(name) => Some(checked(categories::Model::named(db, &name).await?)?.id),
                    None => None,
                };
                let items = watchlist_items::Model::list(db, category).await?;
                json!(items.iter().map(api::watchlist_item).collect::<Vec<_>>())
            }
            "add_watchlist_item" => {
                let params: watchlist_items::Params = parse(arguments)?;
                let item = watchlist_items::Model::create(db, &params).await?;
                api::watchlist_item(&checked(item)?)
            }
            "update_watchlist_item" => {
                let Target { id, rest } = parse::<Target<watchlist_items::Changes>>(arguments)?;
                let item = watchlist_items::Model::change(db, id, rest).await?;
                api::watchlist_item(&checked(item)?)
            }
            "remove_watchlist_item" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                watchlist_items::Model::remove(db, id).await?;
                json!({ "removed": id })
            }
            "list_categories" => {
                let categories = categories::Model::list(db).await?;
                json!(categories.iter().map(api::category).collect::<Vec<_>>())
            }
            "add_category" => {
                let params: categories::Params = parse(arguments)?;
                api::category(&checked(categories::Model::create(db, &params).await?)?)
            }
            "rename_category" => {
                let Target { id, rest } = parse::<Target<categories::Params>>(arguments)?;
                let category = categories::Model::find(db, id).await?;
                api::category(&checked(category.renamed(db, &rest).await?)?)
            }
            "remove_category" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                let category = categories::Model::find(db, id).await?;
                checked(category.removed(db).await?)?;
                json!({ "removed": id })
            }
            "list_schedules" => {
                let schedules = schedules::Model::list(db).await?;
                json!(schedules.iter().map(api::schedule).collect::<Vec<_>>())
            }
            "add_schedule" => {
                let params: schedules::Params = parse(arguments)?;
                api::schedule(&checked(schedules::Model::create(db, &params).await?)?)
            }
            "update_schedule" => {
                let Target { id, rest } = parse::<Target<schedules::Changes>>(arguments)?;
                api::schedule(&checked(schedules::Model::change(db, id, rest).await?)?)
            }
            "remove_schedule" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                schedules::Model::remove(db, id).await?;
                json!({ "removed": id })
            }
            "run_schedule" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                let (schedule, _) = schedules::Model::find(db, id).await?;
                let mut queued = Vec::new();
                for run in runs::Model::queue(db, &schedule).await? {
                    queued.push(self.run(&run, false).await?);
                }
                json!(queued)
            }
            "list_configurations" => {
                let configurations = configurations::Model::list(db).await?;
                json!(configurations
                    .iter()
                    .map(api::configuration)
                    .collect::<Vec<_>>())
            }
            "add_configuration" => {
                let params: configurations::Params = parse(arguments)?;
                let configuration = configurations::Model::create(db, &params).await?;
                api::configuration(&checked(configuration)?)
            }
            "update_configuration" => {
                let Target { id, rest } = parse::<Target<configurations::Changes>>(arguments)?;
                let configuration = configurations::Model::change(db, id, rest).await?;
                api::configuration(&checked(configuration)?)
            }
            "remove_configuration" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                configurations::Model::remove(db, id).await?;
                json!({ "removed": id })
            }
            "activate_configuration" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                let configuration = configurations::Model::find(db, id).await?;
                api::configuration(&configuration.into_active_model().activated(db).await?)
            }
            "deactivate_configuration" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                let configuration = configurations::Model::find(db, id).await?;
                api::configuration(&configuration.into_active_model().deactivated(db).await?)
            }
            "list_runs" => {
                let filter: RunFilter = parse(arguments)?;
                let limit = filter.limit.unwrap_or(RUNS).min(RUNS);
                let mut listed = Vec::new();
                for run in runs::Model::recent(db, limit).await? {
                    listed.push(self.run(&run, false).await?);
                }
                json!(listed)
            }
            "get_run" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                self.run(&runs::Model::find(db, id).await?, true).await?
            }
            other => return Err(Refusal::Failure(format!("the tool {other} does nothing"))),
        })
    }

    /// A run as a client reads it: `whole` adds its prompt and what it printed.
    async fn run(&self, run: &runs::Model, whole: bool) -> std::result::Result<Value, Refusal> {
        let reported = artifacts::Model::of_run(&self.ctx.db, run.id).await?;
        let pages = Settings::from_context(&self.ctx)?.url;
        Ok(if whole {
            api::run_whole(run, &reported, &pages)
        } else {
            api::run(run, &reported, &pages)
        })
    }
}

/// Who calls: the access guard put it in the HTTP request, and rmcp hands that request on.
fn caller(context: &RequestContext<RoleServer>) -> std::result::Result<Caller, ErrorData> {
    context
        .extensions
        .get::<Parts>()
        .and_then(|parts| parts.extensions.get::<Caller>())
        .cloned()
        .ok_or_else(|| ErrorData::internal_error("the request has no caller", None))
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("m8r", env!("CARGO_PKG_VERSION")).with_title("m8r"),
            )
            .with_instructions(INSTRUCTIONS)
    }

    /// The tools of the resources that the caller's token covers.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        let caller = caller(&context)?;
        let categories: Vec<String> = categories::Model::list(&self.ctx.db)
            .await
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))?
            .into_iter()
            .map(|category| category.name)
            .collect();
        let tools = TOOLS
            .iter()
            .filter(|tool| caller.covers(tool.resource))
            .map(|tool| tool.tool(&categories))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let caller = caller(&context)?;
        // A tool that the token doesn't cover is no tool at all to this caller.
        let known = TOOLS
            .iter()
            .any(|tool| tool.name == request.name && caller.covers(tool.resource));
        if !known {
            let unknown = format!("Unknown tool: {}", request.name);
            return Err(ErrorData::invalid_params(unknown, None));
        }
        let result = match self.call(&request.name, request.arguments).await {
            Ok(answer) => CallToolResult::success(vec![ContentBlock::text(answer.to_string())]),
            // The model reads why, and can try again.
            Err(Refusal::Problem(why)) => CallToolResult::error(vec![ContentBlock::text(why)]),
            Err(Refusal::Failure(why)) => return Err(ErrorData::internal_error(why, None)),
        };
        Ok(result.into())
    }

    /// For rmcp's own checks, which know no caller and need no categories.
    fn get_tool(&self, name: &str) -> Option<Tool> {
        let tool = TOOLS.iter().find(|tool| tool.name == name)?;
        Some(tool.tool(&[]))
    }
}

pub fn routes(ctx: &AppContext) -> Routes {
    let config = StreamableHttpServerConfig::default()
        // No sessions: nothing is kept between two requests.
        .with_legacy_session_mode(false)
        // An answer is JSON, not a stream of one event.
        .with_json_response(true)
        // rmcp's Host check is for a server that lets anyone in. This one wants a token first.
        .disable_allowed_hosts();
    let ctx = ctx.clone();
    let service = StreamableHttpService::new(
        move || Ok(Server { ctx: ctx.clone() }),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Routes::new().add("/mcp", any_service(service))
}
