use std::collections::{BTreeMap, HashSet};

pub use super::_entities::watchlist_items::{ActiveModel, Column, Entity, Model};
use chrono::{DateTime, Utc};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{categories, tool_errors, Checked};
use crate::tools;

pub type WatchlistItems = Entity;

/// How long an item's title and state are good for. An item whose last check failed is checked
/// again at every tick.
const CHECK_EVERY: chrono::Duration = chrono::Duration::minutes(10);

/// How much you care about an item: at all, or urgently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Medium,
    High,
}

impl Priority {
    pub const ALL: [Self; 2] = [Self::Medium, Self::High];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|priority| priority.as_str() == name)
    }
}

/// Where an item's issue or PR/MR lives, read from its URL. The forge is the source of truth
/// for the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forge {
    /// The CLI that talks to this forge: `gh` for github.com, `glab` for any other host.
    pub tool: &'static str,
    pub host: String,
    /// Its path in the forge's REST API.
    pub endpoint: String,
    /// A short name for it, such as `rgnets/rxg!4521`.
    pub label: String,
    /// It's a PR/MR, which someone can ask you to review. An issue isn't.
    pub reviewable: bool,
}

/// What the forge says about an item's issue or PR/MR.
#[derive(Debug, PartialEq, Eq)]
pub enum Remote {
    Open {
        title: String,
    },
    /// Closed or merged: done.
    Closed,
}

impl Forge {
    pub const TOOLS: [&'static str; 2] = ["gh", "glab"];

    /// Read the URL of an issue or a PR/MR on GitHub or GitLab.
    #[must_use]
    pub fn parse(url: &str) -> Option<Self> {
        let rest = url
            .trim()
            .strip_prefix("https://")
            .or_else(|| url.trim().strip_prefix("http://"))?;
        let rest = rest.split(['?', '#']).next()?;
        let (host, path) = rest.split_once('/')?;
        let plain = |text: &str| {
            !text.is_empty()
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        };
        if !host.split(':').all(plain) {
            return None;
        }
        let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        fn number(text: &str) -> Option<&str> {
            (!text.is_empty() && text.chars().all(|c| c.is_ascii_digit())).then_some(text)
        }

        if host == "github.com" {
            let [owner, repo, kind, number_text, ..] = segments[..] else {
                return None;
            };
            let number = number(number_text)?;
            if !plain(owner) || !plain(repo) || !["issues", "pull"].contains(&kind) {
                return None;
            }
            // GitHub's issues API answers for pull requests too.
            return Some(Self {
                tool: "gh",
                host: host.to_string(),
                endpoint: format!("repos/{owner}/{repo}/issues/{number}"),
                label: format!("{owner}/{repo}#{number}"),
                reviewable: kind == "pull",
            });
        }

        // GitLab: <group>/<subgroups>/<project>/-/<kind>/<number>
        let at = segments
            .iter()
            .rposition(|part| ["issues", "work_items", "merge_requests"].contains(part))?;
        let number = number(segments.get(at + 1)?)?;
        let project = segments[..at]
            .strip_suffix(&["-"])
            .unwrap_or(&segments[..at]);
        if project.len() < 2 || !project.iter().all(|part| plain(part)) {
            return None;
        }
        let reviewable = segments[at] == "merge_requests";
        let (kind, mark) = if reviewable {
            ("merge_requests", '!')
        } else {
            ("issues", '#')
        };
        Some(Self {
            tool: "glab",
            host: host.to_string(),
            endpoint: format!("projects/{}/{kind}/{number}", project.join("%2F")),
            label: format!("{}{mark}{number}", project.join("/")),
            reviewable,
        })
    }

    /// The command that asks the forge about the issue or PR/MR.
    #[must_use]
    pub fn command(&self) -> Vec<String> {
        [self.tool, "api", "--hostname", &self.host, &self.endpoint]
            .map(str::to_string)
            .to_vec()
    }

    /// Read the forge's answer: the issue or PR/MR as JSON.
    #[must_use]
    pub fn read(answer: &str) -> Option<Remote> {
        // A login shell may print something of its own first.
        let json = answer.get(answer.find('{')?..)?;
        let found = serde_json::Deserializer::from_str(json)
            .into_iter::<serde_json::Value>()
            .next()?
            .ok()?;
        let title = found.get("title")?.as_str()?;
        // GitHub: open, closed. GitLab: opened, closed, merged, locked.
        match found.get("state")?.as_str()? {
            "closed" | "merged" => Some(Remote::Closed),
            _ => Some(Remote::Open {
                title: title.to_string(),
            }),
        }
    }
}

/// What the watchlist's forms and the API send.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Params {
    pub url: String,
    /// The category's name.
    pub category: String,
    /// Medium, unless it says high.
    pub priority: String,
    pub note: String,
    /// A checkbox, which a form sends only when it's checked.
    pub review_requested: bool,
}

/// What changes on an item: only what's there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Changes {
    pub category: Option<String>,
    pub priority: Option<String>,
    pub note: Option<String>,
    pub review_requested: Option<bool>,
}

impl Params {
    /// The item as it is, with `changes`.
    #[must_use]
    pub fn changed((item, category): &Listed, changes: Changes) -> Self {
        Self {
            url: item.url.clone(),
            category: changes.category.unwrap_or_else(|| category.name.clone()),
            priority: changes.priority.unwrap_or_else(|| item.priority.clone()),
            note: changes.note.unwrap_or_else(|| item.note.clone()),
            review_requested: changes.review_requested.unwrap_or(item.review_requested),
        }
    }

    fn priority(&self) -> &str {
        if self.priority.trim().is_empty() {
            return Priority::Medium.as_str();
        }
        self.priority.trim()
    }

    /// Why these can't be saved, if they can't.
    fn problem(&self) -> Option<String> {
        if Priority::parse(self.priority()).is_none() {
            return Some("The priority is medium or high.".to_string());
        }
        let Some(forge) = Forge::parse(&self.url) else {
            return Some(
                "That isn't the URL of an issue or a PR/MR on GitHub or GitLab.".to_string(),
            );
        };
        if self.review_requested && !forge.reviewable {
            return Some(
                "Only a PR/MR can have its review requested. That's an issue.".to_string(),
            );
        }
        None
    }

    /// The category that these name, or why they can't be saved.
    async fn checked<C: ConnectionTrait>(&self, db: &C) -> ModelResult<Checked<categories::Model>> {
        if let Some(problem) = self.problem() {
            return Ok(Err(problem));
        }
        categories::Model::named(db, &self.category).await
    }
}

/// An item and its category: what the pages, the API and the prompts show.
pub type Listed = (Model, categories::Model);

/// Why a forge's items couldn't all be checked.
enum Failure {
    /// The tool itself: it isn't installed, or can't start.
    Tool(String),
    /// The tool's dealings with this host: its login, the network, or one issue or PR/MR.
    Host(String),
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if !insert && self.updated_at.is_unchanged() {
            let mut this = self;
            this.updated_at = sea_orm::ActiveValue::Set(chrono::Utc::now().into());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

impl Model {
    /// The watchlist, or with `category_id`, that category's part of it. Category by category,
    /// the urgent items first.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list(
        db: &DatabaseConnection,
        category_id: Option<i64>,
    ) -> ModelResult<Vec<Listed>> {
        let mut items = Entity::find().find_also_related(categories::Entity);
        if let Some(category_id) = category_id {
            items = items.filter(Column::CategoryId.eq(category_id));
        }
        let items = items
            .order_by_asc(Column::CategoryId)
            // "high" sorts before "medium".
            .order_by_asc(Column::Priority)
            .order_by_asc(Column::Id)
            .all(db)
            .await?;
        // Every item has a category: the database sees to it.
        Ok(items
            .into_iter()
            .filter_map(|(item, category)| Some((item, category?)))
            .collect())
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such item.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Listed> {
        let item = Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;
        let category = categories::Model::find(db, item.category_id).await?;
        Ok((item, category))
    }

    /// Put an issue or a PR/MR on the watchlist. One that's already there gets the new
    /// category, priority, note and review request.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Checked<Listed>> {
        let url = params.url.trim();
        let txn = db.begin().await?;
        let category = match params.checked(&txn).await? {
            Ok(category) => category,
            Err(problem) => return Ok(Err(problem)),
        };
        let listed = Entity::find().filter(Column::Url.eq(url)).one(&txn).await?;
        let item = if let Some(item) = listed {
            let mut item = item.into_active_model();
            item.set(params, &category);
            item.update(&txn).await?
        } else {
            let mut item = ActiveModel::new();
            item.url = Set(url.to_string());
            item.set(params, &category);
            item.insert(&txn).await?
        };
        txn.commit().await?;
        Ok(Ok((item, category)))
    }

    /// Change the item: `params` are all that it has from you.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn revised(
        self,
        db: &DatabaseConnection,
        params: &Params,
    ) -> ModelResult<Checked<Listed>> {
        let category = match params.checked(db).await? {
            Ok(category) => category,
            Err(problem) => return Ok(Err(problem)),
        };
        let mut item = self.into_active_model();
        item.set(params, &category);
        Ok(Ok((item.update(db).await?, category)))
    }

    /// Change item `id`: only what `changes` has.
    ///
    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such item, or when a query fails.
    pub async fn change(
        db: &DatabaseConnection,
        id: i64,
        changes: Changes,
    ) -> ModelResult<Checked<Listed>> {
        let listed = Self::find(db, id).await?;
        let params = Params::changed(&listed, changes);
        listed.0.revised(db, &params).await
    }

    /// Take item `id` off the watchlist.
    ///
    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such item, or when a query fails.
    pub async fn remove(db: &DatabaseConnection, id: i64) -> ModelResult<()> {
        Self::find(db, id).await?.0.delete(db).await?;
        Ok(())
    }

    /// A short name for the item, such as `rgnets/rxg!4521`.
    #[must_use]
    pub fn label(&self) -> String {
        Forge::parse(&self.url).map_or_else(|| self.url.clone(), |forge| forge.label)
    }

    /// It's a PR/MR, which someone can ask you to review. An issue isn't.
    #[must_use]
    pub fn reviewable(&self) -> bool {
        Forge::parse(&self.url).is_some_and(|forge| forge.reviewable)
    }

    /// Ask the forges about the items that are due, through `gh` and `glab`. An item whose
    /// issue or PR/MR closed or merged leaves the watchlist. What's wrong with a tool goes in
    /// `tool_errors`, and leaves it when the tool works again.
    ///
    /// # Errors
    /// When a query fails. A tool that fails is not an error here.
    pub async fn refresh(db: &DatabaseConnection, shell: &str) -> ModelResult<()> {
        let now = Utc::now();
        let failing: HashSet<(String, String)> = tool_errors::Model::list(db)
            .await?
            .into_iter()
            .map(|error| (error.tool, error.host))
            .collect();
        let mut forges: BTreeMap<(&'static str, String), Vec<(Self, Forge)>> = BTreeMap::new();
        for item in Entity::find().order_by_asc(Column::Id).all(db).await? {
            if let Some(forge) = Forge::parse(&item.url) {
                let key = (forge.tool, forge.host.clone());
                forges.entry(key).or_default().push((item, forge));
            }
        }

        for ((tool, host), items) in &forges {
            let stale = |item: &Self| {
                item.checked_at
                    .is_none_or(|at| now - at.with_timezone(&Utc) > CHECK_EVERY)
            };
            let due = failing.contains(&((*tool).to_string(), host.clone()))
                || failing.contains(&((*tool).to_string(), String::new()))
                || items.iter().any(|(item, _)| stale(item));
            if !due {
                continue;
            }
            match Self::check(db, shell, items, now).await? {
                Some(Failure::Tool(message)) => {
                    tool_errors::Model::set(db, tool, "", &message).await?;
                    tool_errors::Model::clear(db, tool, host).await?;
                }
                Some(Failure::Host(message)) => {
                    tool_errors::Model::set(db, tool, host, &message).await?;
                    tool_errors::Model::clear(db, tool, "").await?;
                }
                None => {
                    tool_errors::Model::clear(db, tool, host).await?;
                    tool_errors::Model::clear(db, tool, "").await?;
                }
            }
        }

        // An error about a forge that no item is on any more has nothing left to clear it.
        for error in tool_errors::Model::list(db).await? {
            let wanted = forges.keys().any(|(tool, host)| {
                *tool == error.tool && (error.host.is_empty() || *host == error.host)
            });
            if Forge::TOOLS.contains(&error.tool.as_str()) && !wanted {
                tool_errors::Model::clear(db, &error.tool, &error.host).await?;
            }
        }
        Ok(())
    }

    /// Check one forge's items, and say what failed first, if anything did.
    async fn check(
        db: &DatabaseConnection,
        shell: &str,
        items: &[(Self, Forge)],
        now: DateTime<Utc>,
    ) -> ModelResult<Option<Failure>> {
        let mut failure = None;
        for (item, forge) in items {
            let output = match tools::run(shell, &forge.command(), tools::ANSWER_LIMIT).await {
                Ok(output) => output,
                Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
                    failure.get_or_insert(Failure::Host(format!("{}: {err}", forge.label)));
                    continue;
                }
                Err(err) => {
                    return Ok(Some(Failure::Tool(format!("m8r couldn't run it: {err}"))));
                }
            };
            if output.missing() {
                return Ok(Some(Failure::Tool(
                    "It isn't installed: the login shell of m8r's user finds no such command."
                        .to_string(),
                )));
            }
            if !output.ok() {
                let message = format!("{}: {}", forge.label, output.complaint());
                failure.get_or_insert(Failure::Host(message));
                continue;
            }
            match Forge::read(&output.stdout) {
                Some(Remote::Open { title }) => {
                    item.clone()
                        .into_active_model()
                        .checked(db, title, now)
                        .await?;
                }
                Some(Remote::Closed) => {
                    item.clone().delete(db).await?;
                }
                None => {
                    let message = format!("{}: m8r couldn't read the answer.", forge.label);
                    failure.get_or_insert(Failure::Host(message));
                }
            }
        }
        Ok(failure)
    }
}

impl ActiveModel {
    fn set(&mut self, params: &Params, category: &categories::Model) {
        self.category_id = Set(category.id);
        self.priority = Set(params.priority().to_string());
        self.note = Set(params.note.trim().to_string());
        self.review_requested = Set(params.review_requested);
    }

    /// Record what the forge said.
    ///
    /// # Errors
    /// When the update fails.
    pub async fn checked(
        mut self,
        db: &DatabaseConnection,
        title: String,
        at: DateTime<Utc>,
    ) -> ModelResult<Model> {
        self.title = Set(title);
        self.checked_at = Set(Some(at.into()));
        Ok(self.update(db).await?)
    }
}
