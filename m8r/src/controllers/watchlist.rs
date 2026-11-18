#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    models::{
        repos,
        watchlist_items::{self, Category, Filter, Params},
    },
    views,
};

/// Which part of the watchlist the page shows: what its address says after the `?`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Shown {
    /// A category's name. Blank, or anything else: all of them.
    pub category: String,
    /// A repo's id. Blank, or anything else: all of them.
    pub repo_id: String,
}

impl Shown {
    fn filter(&self) -> Filter {
        Filter {
            repo_id: self.repo_id.trim().parse().ok(),
            category: Category::parse(self.category.trim()),
        }
    }
}

/// `text` as an address holds it after the `?`.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The watchlist's page, with `form` in its form for a new item, and what's wrong, if
/// something is.
async fn page(
    v: &TeraView,
    ctx: &AppContext,
    shown: &Shown,
    form: &Params,
    problem: Option<String>,
) -> Result<Response> {
    let filter = shown.filter();
    let items: Vec<_> = watchlist_items::Model::list(&ctx.db, filter)
        .await?
        .iter()
        .map(views::watchlist_item)
        .collect();
    // A tab for all of them, and one for each category. A tab keeps the repo that's shown.
    let of_repo = filter.repo_id.map(|id| format!("repo_id={id}"));
    let tab = |category: Option<Category>| {
        let name = category.map_or("all", Category::as_str);
        let of_category = category.map(|it| format!("category={}", encoded(it.as_str())));
        let query: Vec<String> = of_category.into_iter().chain(of_repo.clone()).collect();
        let href = if query.is_empty() {
            "/watchlist".to_string()
        } else {
            format!("/watchlist?{}", query.join("&"))
        };
        json!({ "name": name, "href": href, "current": category == filter.category })
    };
    let tabs: Vec<Value> = std::iter::once(None)
        .chain(Category::ALL.map(Some))
        .map(tab)
        .collect();
    let repos: Vec<Value> = repos::Model::list(&ctx.db)
        .await?
        .iter()
        .map(|repo| {
            let shown = Some(repo.id) == filter.repo_id;
            json!({ "id": repo.id, "path": repo.path, "shown": shown })
        })
        .collect();
    let data = json!({
        "items": items,
        "tabs": tabs,
        "repos": repos,
        "category": filter.category.map(Category::as_str).unwrap_or_default(),
        "form": form,
        "has_problem": problem.is_some(),
        "problem": problem.unwrap_or_default(),
    });
    views::page(v, ctx, "watchlist/index.html", "watchlist", data).await
}

/// The watchlist: the issues and PRs/MRs that you follow, each in one of m8r's repos.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Query(shown): Query<Shown>,
) -> Result<Response> {
    page(&v, &ctx, &shown, &Params::default(), None).await
}

#[debug_handler]
pub async fn add(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(params): Form<Params>,
) -> Result<Response> {
    match watchlist_items::Model::create(&ctx.db, &params).await? {
        Ok(_) => format::redirect("/watchlist"),
        Err(problem) => page(&v, &ctx, &Shown::default(), &params, Some(problem)).await,
    }
}

#[debug_handler]
pub async fn update(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
    Form(params): Form<Params>,
) -> Result<Response> {
    // A row's form sends no URL: an item keeps its own.
    let (item, _) = watchlist_items::Model::find(&ctx.db, id).await?;
    match item.revised(&ctx.db, &params).await? {
        Ok(_) => format::redirect("/watchlist"),
        Err(problem) => {
            let form = Params::default();
            page(&v, &ctx, &Shown::default(), &form, Some(problem)).await
        }
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
