//! The MCP endpoint, `/mcp`: the watchlist as tools, for an agent that isn't one of m8r's runs,
//! such as Claude Duet. It keeps the watchlist, and it reads the repos and the runs. It
//! changes nothing else: the API has the rest.
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
    models::{
        repos, runs,
        watchlist_items::{self, Category, Priority},
        Checked,
    },
    views::api,
};

/// What every client reads first.
const INSTRUCTIONS: &str = "\
m8r keeps the developer's watchlist: the issues and PRs/MRs that they follow, which m8r's \
coding agents work on. Your part is to keep that watchlist true: add what the developer must \
follow, turn requested on when someone asks them for it, and remove what's done. A watchlist \
item is a URL on GitHub or GitLab, which says which repo the item is in and whether it's an \
issue or a PR/MR. It has a priority (medium: the developer cares about it, and it's what an \
item gets when nobody says; high: it matters more; urgent: it can't wait), a note that says \
why, and whether someone asked the developer for it (requested): for a PR/MR, a review. An \
agent gets the items of the repo that it runs in: the requested ones first, then the most \
pressing ones. m8r takes an item only for a repo that it works in, and removes an item by \
itself when its issue or PR/MR closes or merges. You can also list those repos, and read the \
agents' runs with what they reported. You change neither: the developer does, in m8r's pages.";

/// What a tool does to m8r, which the client's approval step goes by.
#[derive(Clone, Copy)]
enum Effect {
    Reads,
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

const TOOLS: [Spec; 7] = [
    spec(
        "list_watchlist_items",
        "List the watchlist",
        "watchlist_items",
        Effect::Reads,
        "List the watchlist: the issues and PRs/MRs that the developer follows, or only a \
         repo's or a category's. Each item has an id, a url, its repo, its category, a \
         priority, a note, requested, and the title that the forge gives it.",
    ),
    spec(
        "add_watchlist_item",
        "Add a watchlist item",
        "watchlist_items",
        Effect::Sets,
        "Put an issue or a PR/MR on the watchlist, by its URL on GitHub or GitLab. With no \
         priority, it's medium. The URL says which repo it's in, and m8r refuses one whose repo \
         it doesn't have: only the developer adds a repo, in m8r's pages. If the URL is already \
         there, this changes that item.",
    ),
    spec(
        "update_watchlist_item",
        "Change a watchlist item",
        "watchlist_items",
        Effect::Sets,
        "Change a watchlist item: its priority, note or requested. What you leave out stays \
         as it is.",
    ),
    spec(
        "remove_watchlist_item",
        "Remove a watchlist item",
        "watchlist_items",
        Effect::Removes,
        "Take an item off the watchlist, when the developer no longer follows it.",
    ),
    spec(
        "list_repos",
        "List the repos",
        "repos",
        Effect::Reads,
        "List the repos that m8r works in: a watchlist item is in one of them. Each has an id, \
         the path of its clone in the home of m8r's user, and the URL of its project on its \
         forge.",
    ),
    spec(
        "list_runs",
        "List the runs",
        "runs",
        Effect::Reads,
        "List the latest runs, newest first: the queue and the history, each with its status.",
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
    /// The tool as a client sees it.
    fn tool(&self) -> Tool {
        let hints = ToolAnnotations::with_title(self.title).open_world(false);
        let hints = match self.effect {
            Effect::Reads => hints.read_only(true),
            Effect::Sets => hints.read_only(false).destructive(false).idempotent(true),
            Effect::Removes => hints.read_only(false).destructive(true).idempotent(true),
        };
        Tool::new(self.name, self.description, self.input())
            .with_title(self.title)
            .with_annotations(hints)
    }

    /// The JSON schema of the tool's arguments.
    fn input(&self) -> JsonObject {
        let id = |of: &str| {
            let description = format!("The id of the {of}, as its list shows it.");
            json!({ "type": "integer", "description": description })
        };
        let text = |description: &str| json!({ "type": "string", "description": description });
        let category = json!({
            "type": "string",
            "enum": Category::ALL.map(Category::as_str),
            "description": "Only the items of this category.",
        });
        let priority = json!({
            "type": "string",
            "enum": Priority::ALL.map(Priority::as_str),
            "description": "medium: the developer cares about it. high: it matters more. \
                            urgent: it can't wait.",
        });
        let note = text("Why the developer follows it, in a few words.");
        let requested = json!({
            "type": "boolean",
            "description": "Someone asked the developer for it: for a PR/MR, a review.",
        });

        let (properties, required): (Value, &[&str]) = match self.name {
            "list_watchlist_items" => (
                json!({ "category": category, "repo_id": id("repo whose items to list") }),
                &[],
            ),
            "add_watchlist_item" => (
                json!({
                    "url": text("The URL of the issue or PR/MR."),
                    "priority": priority,
                    "note": note,
                    "requested": requested,
                }),
                &["url"],
            ),
            "update_watchlist_item" => (
                json!({
                    "id": id("item"),
                    "priority": priority,
                    "note": note,
                    "requested": requested,
                }),
                &["id"],
            ),
            "remove_watchlist_item" => (json!({ "id": id("item") }), &["id"]),
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
    repo_id: Option<i64>,
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
                let filter = watchlist_items::Filter {
                    repo_id: filter.repo_id,
                    category: checked(Category::named(filter.category.as_deref()))?,
                };
                let items = watchlist_items::Model::list(db, filter).await?;
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
            "list_repos" => {
                let repos = repos::Model::list(db).await?;
                json!(repos.iter().map(api::repo).collect::<Vec<_>>())
            }
            "list_runs" => {
                let filter: RunFilter = parse(arguments)?;
                let limit = filter.limit.unwrap_or(RUNS).min(RUNS);
                let listed = runs::Model::recent(db, limit).await?;
                json!(listed.iter().map(api::run).collect::<Vec<_>>())
            }
            "get_run" => {
                let Target { id, .. } = parse::<Target<Nothing>>(arguments)?;
                api::run_whole(&runs::Model::find(db, id).await?)
            }
            other => return Err(Refusal::Failure(format!("the tool {other} does nothing"))),
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
        let tools = TOOLS
            .iter()
            .filter(|tool| caller.covers(tool.resource))
            .map(Spec::tool)
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

    /// For rmcp's own checks, which know no caller.
    fn get_tool(&self, name: &str) -> Option<Tool> {
        let tool = TOOLS.iter().find(|tool| tool.name == name)?;
        Some(tool.tool())
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
