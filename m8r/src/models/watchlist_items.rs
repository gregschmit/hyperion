use std::collections::{BTreeMap, HashSet};

pub use super::_entities::watchlist_items::{ActiveModel, Column, Entity, Model};
use chrono::{DateTime, Utc};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{errors, repos, Checked};
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

/// What an item is: an issue, or a PR/MR. Its URL says which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Issues,
    Requests,
}

impl Category {
    pub const ALL: [Self; 2] = [Self::Issues, Self::Requests];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Issues => "issues",
            Self::Requests => "pull/merge requests",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == name)
    }

    /// The category that `name` names, or with no name, none. Or why not: the API and the MCP
    /// tools say that back.
    ///
    /// # Errors
    /// A message that names the categories.
    pub fn named(name: Option<&str>) -> Checked<Option<Self>> {
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok(None);
        };
        let names = Self::ALL.map(Self::as_str).join(", ");
        Self::parse(name)
            .map(Some)
            .ok_or_else(|| format!("There's no category \"{name}\". The categories are: {names}."))
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
    /// Its project's own URL, such as `https://git.example.com/group/project`, which says
    /// which repo the item is in (see `repos::project_url`).
    pub project: String,
    pub category: Category,
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
        let host = host.to_ascii_lowercase();
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
            let category = match kind {
                "pull" => Category::Requests,
                _ => Category::Issues,
            };
            // GitHub's issues API answers for pull requests too.
            return Some(Self {
                tool: "gh",
                endpoint: format!("repos/{owner}/{repo}/issues/{number}"),
                label: format!("{owner}/{repo}#{number}"),
                project: format!("https://{host}/{owner}/{repo}"),
                category,
                host,
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
        let (category, kind, mark) = match segments[at] {
            "merge_requests" => (Category::Requests, "merge_requests", '!'),
            _ => (Category::Issues, "issues", '#'),
        };
        Some(Self {
            tool: "glab",
            endpoint: format!("projects/{}/{kind}/{number}", project.join("%2F")),
            label: format!("{}{mark}{number}", project.join("/")),
            project: format!("https://{host}/{}", project.join("/")),
            category,
            host,
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

/// What the watchlist's forms and the API send. The URL says the rest: which repo the item is
/// in, and whether it's an issue or a PR/MR.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Params {
    pub url: String,
    /// Medium, unless it says high.
    pub priority: String,
    pub note: String,
    /// Someone asked you for it: a review, if it's a PR/MR. A checkbox, which a form sends
    /// only when it's checked.
    pub requested: bool,
}

/// What changes on an item: only what's there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Changes {
    pub priority: Option<String>,
    pub note: Option<String>,
    pub requested: Option<bool>,
}

impl Params {
    /// The item as it is, with `changes`.
    #[must_use]
    pub fn changed(item: &Model, changes: Changes) -> Self {
        Self {
            url: item.url.clone(),
            priority: changes.priority.unwrap_or_else(|| item.priority.clone()),
            note: changes.note.unwrap_or_else(|| item.note.clone()),
            requested: changes.requested.unwrap_or(item.requested),
        }
    }

    fn priority(&self) -> &str {
        if self.priority.trim().is_empty() {
            return Priority::Medium.as_str();
        }
        self.priority.trim()
    }

    /// Where the URL says the item is, and the repo that it's in. Or why these can't be
    /// saved: an item is in a repo that m8r has.
    async fn checked<C: ConnectionTrait>(
        &self,
        db: &C,
    ) -> ModelResult<Checked<(Forge, repos::Model)>> {
        if Priority::parse(self.priority()).is_none() {
            return Ok(Err("The priority is medium or high.".to_string()));
        }
        let Some(forge) = Forge::parse(&self.url) else {
            return Ok(Err(
                "That isn't the URL of an issue or a PR/MR on GitHub or GitLab.".to_string(),
            ));
        };
        let repos = repos::Entity::find().all(db).await?;
        let Some(repo) = repos.into_iter().find(|repo| repo.is(&forge.project)) else {
            return Ok(Err(format!(
                "m8r has no repo for {}. Add that repo first: an item is in one.",
                forge.project
            )));
        };
        Ok(Ok((forge, repo)))
    }
}

/// Which items a list holds: all of them, or only a repo's, or only a category's.
#[derive(Debug, Clone, Copy, Default)]
pub struct Filter {
    pub repo_id: Option<i64>,
    pub category: Option<Category>,
}

/// An item and its repo: what the pages, the API and the MCP tools show.
pub type Listed = (Model, repos::Model);

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
    /// The watchlist, or the part of it that `filter` names. Repo by repo, and in each, the
    /// requested items first, then the urgent ones.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection, filter: Filter) -> ModelResult<Vec<Listed>> {
        let mut items = Entity::find().find_also_related(repos::Entity);
        if let Some(repo_id) = filter.repo_id {
            items = items.filter(Column::RepoId.eq(repo_id));
        }
        if let Some(category) = filter.category {
            items = items.filter(Column::Category.eq(category.as_str()));
        }
        let items = items
            .order_by_asc(repos::Column::Path)
            .order_by_desc(Column::Requested)
            // "high" sorts before "medium".
            .order_by_asc(Column::Priority)
            .order_by_asc(Column::Id)
            .all(db)
            .await?;
        // Every item has a repo: the database sees to it.
        Ok(items
            .into_iter()
            .filter_map(|(item, repo)| Some((item, repo?)))
            .collect())
    }

    /// The items that a run in repo `repo_id` gets: the repo's, of `category` or of both. The
    /// requested ones come first, then the urgent ones, and `most` cuts the list there (0:
    /// all of them).
    ///
    /// # Errors
    /// When the query fails.
    pub async fn of_repo(
        db: &DatabaseConnection,
        repo_id: i64,
        category: Option<Category>,
        most: u64,
    ) -> ModelResult<Vec<Self>> {
        let mut items = Entity::find().filter(Column::RepoId.eq(repo_id));
        if let Some(category) = category {
            items = items.filter(Column::Category.eq(category.as_str()));
        }
        let items = items
            .order_by_desc(Column::Requested)
            .order_by_asc(Column::Priority)
            .order_by_asc(Column::Id);
        Ok(match most {
            0 => items.all(db).await?,
            most => items.limit(most).all(db).await?,
        })
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such item.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Listed> {
        let item = Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;
        let repo = repos::Model::find(db, item.repo_id).await?;
        Ok((item, repo))
    }

    /// Put an issue or a PR/MR on the watchlist. One that's already there gets the new
    /// priority, note and request.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Checked<Listed>> {
        let url = params.url.trim();
        let txn = db.begin().await?;
        let (forge, repo) = match params.checked(&txn).await? {
            Ok(checked) => checked,
            Err(problem) => return Ok(Err(problem)),
        };
        let listed = Entity::find().filter(Column::Url.eq(url)).one(&txn).await?;
        let is_new = listed.is_none();
        let mut item = listed.map_or_else(ActiveModel::new, IntoActiveModel::into_active_model);
        item.url = Set(url.to_string());
        item.repo_id = Set(repo.id);
        item.category = Set(forge.category.as_str().to_string());
        item.set(params);
        let item = if is_new {
            item.insert(&txn).await?
        } else {
            item.update(&txn).await?
        };
        txn.commit().await?;
        Ok(Ok((item, repo)))
    }

    /// Change the item: `params` are all that it has from you. Its URL stays, and so do its
    /// repo and its category, which the URL says.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn revised(
        self,
        db: &DatabaseConnection,
        params: &Params,
    ) -> ModelResult<Checked<Self>> {
        if Priority::parse(params.priority()).is_none() {
            return Ok(Err("The priority is medium or high.".to_string()));
        }
        let mut item = self.into_active_model();
        item.set(params);
        Ok(Ok(item.update(db).await?))
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
        let (item, repo) = Self::find(db, id).await?;
        let params = Params::changed(&item, changes);
        Ok(item.revised(db, &params).await?.map(|item| (item, repo)))
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

    /// Ask the forges about the items that are due, through `gh` and `glab`. An item whose
    /// issue or PR/MR closed or merged leaves the watchlist. A tool that fails raises an
    /// error, and the same check clears it when the tool works again, or when no item needs
    /// the tool any more.
    ///
    /// # Errors
    /// When a query fails. A tool that fails is not an error here.
    pub async fn refresh(db: &DatabaseConnection, shell: &str) -> ModelResult<()> {
        let now = Utc::now();
        let failing: HashSet<String> = errors::Model::open(db)
            .await?
            .into_iter()
            .map(|error| error.subject)
            .collect();
        let mut forges: BTreeMap<(&'static str, String), Vec<(Self, Forge)>> = BTreeMap::new();
        for item in Entity::find().order_by_asc(Column::Id).all(db).await? {
            if let Some(forge) = Forge::parse(&item.url) {
                let key = (forge.tool, forge.host.clone());
                forges.entry(key).or_default().push((item, forge));
            }
        }

        for ((tool, host), items) in &forges {
            // The tool's own error, and the error of the tool on this host.
            let (missing, refused) = ((*tool).to_string(), format!("{tool} on {host}"));
            let stale = |item: &Self| {
                item.checked_at
                    .is_none_or(|at| now - at.with_timezone(&Utc) > CHECK_EVERY)
            };
            let due = failing.contains(&missing)
                || failing.contains(&refused)
                || items.iter().any(|(item, _)| stale(item));
            if !due {
                continue;
            }
            match Self::check(db, shell, items, now).await? {
                Some(Failure::Tool(message)) => {
                    errors::Model::raise(db, &missing, &message, None).await?;
                    errors::Model::clear(db, &refused).await?;
                }
                Some(Failure::Host(message)) => {
                    errors::Model::raise(db, &refused, &message, None).await?;
                    errors::Model::clear(db, &missing).await?;
                }
                None => {
                    errors::Model::clear(db, &refused).await?;
                    errors::Model::clear(db, &missing).await?;
                }
            }
        }

        // An error about a forge that no item is on any more has nothing left to clear it.
        for subject in failing {
            let mut words = subject.split(" on ");
            let (tool, host) = (words.next().unwrap_or_default(), words.next());
            let wanted = forges
                .keys()
                .any(|(wanted, on)| *wanted == tool && host.is_none_or(|host| on == host));
            if Forge::TOOLS.contains(&tool) && !wanted {
                errors::Model::clear(db, &subject).await?;
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
                    let message = format!("m8r couldn't run {}: {err}", forge.tool);
                    return Ok(Some(Failure::Tool(message)));
                }
            };
            if output.missing() {
                return Ok(Some(Failure::Tool(format!(
                    "m8r asks the forge about the watchlist with {}, and the login shell of \
                     m8r's user finds no such command.",
                    forge.tool
                ))));
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
    fn set(&mut self, params: &Params) {
        self.priority = Set(params.priority().to_string());
        self.note = Set(params.note.trim().to_string());
        self.requested = Set(params.requested);
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
