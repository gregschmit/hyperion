use std::collections::{BTreeMap, HashSet};

pub use super::_entities::items::{ActiveModel, Column, Entity, Model};
use chrono::{DateTime, Utc};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::tool_errors;
use crate::tools;

pub type Items = Entity;

/// How long an item's title and state are good for. An item whose last check failed is
/// checked again at every tick.
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

/// Where an item lives, read from its URL. The forge is the source of truth for the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forge {
    /// The CLI that talks to this forge: `gh` for github.com, `glab` for any other host.
    pub tool: &'static str,
    pub host: String,
    /// The item's path in the forge's REST API.
    pub endpoint: String,
    /// A short name for the item, such as `rgnets/rxg!4521`.
    pub label: String,
}

/// What the forge says about an item.
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
        let (kind, mark) = match segments[at] {
            "merge_requests" => ("merge_requests", '!'),
            _ => ("issues", '#'),
        };
        Some(Self {
            tool: "glab",
            host: host.to_string(),
            endpoint: format!("projects/{}/{kind}/{number}", project.join("%2F")),
            label: format!("{}{mark}{number}", project.join("/")),
        })
    }

    /// The command that asks the forge about the item.
    #[must_use]
    pub fn command(&self) -> Vec<String> {
        [self.tool, "api", "--hostname", &self.host, &self.endpoint]
            .map(str::to_string)
            .to_vec()
    }

    /// Read the forge's answer: the item as JSON.
    #[must_use]
    pub fn read(answer: &str) -> Option<Remote> {
        // A login shell may print something of its own first.
        let json = answer.get(answer.find('{')?..)?;
        let item = serde_json::Deserializer::from_str(json)
            .into_iter::<serde_json::Value>()
            .next()?
            .ok()?;
        let title = item.get("title")?.as_str()?;
        // GitHub: open, closed. GitLab: opened, closed, merged, locked.
        match item.get("state")?.as_str()? {
            "closed" | "merged" => Some(Remote::Closed),
            _ => Some(Remote::Open {
                title: title.to_string(),
            }),
        }
    }
}

/// What an item's form sends.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Params {
    #[serde(default)]
    pub url: String,
    pub priority: String,
    pub note: String,
}

impl Params {
    /// Why these can't be saved, if they can't.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if Priority::parse(&self.priority).is_none() {
            return Some("Choose medium or high.".to_string());
        }
        if Forge::parse(&self.url).is_none() {
            return Some(
                "That isn't the URL of an issue or a PR/MR on GitHub or GitLab.".to_string(),
            );
        }
        None
    }
}

/// Why a forge's items couldn't all be checked.
enum Failure {
    /// The tool itself: it isn't installed, or can't start.
    Tool(String),
    /// The tool's dealings with this host: its login, the network, or one item.
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
    /// Every item, the urgent ones first.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            // "high" sorts before "medium".
            .order_by_asc(Column::Priority)
            .order_by_asc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such item.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Add an item. One that's already listed gets the new priority and note.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Self> {
        let url = params.url.trim();
        let txn = db.begin().await?;
        let listed = Entity::find().filter(Column::Url.eq(url)).one(&txn).await?;
        let item = match listed {
            Some(item) => item.into_active_model().revised(&txn, params).await?,
            None => {
                let mut item = ActiveModel::new();
                item.url = Set(url.to_string());
                item.priority = Set(params.priority.clone());
                item.note = Set(params.note.trim().to_string());
                item.insert(&txn).await?
            }
        };
        txn.commit().await?;
        Ok(item)
    }

    /// The item's short name, such as `rgnets/rxg!4521`.
    #[must_use]
    pub fn label(&self) -> String {
        Forge::parse(&self.url).map_or_else(|| self.url.clone(), |forge| forge.label)
    }

    /// Ask the forges about the items that are due, through `gh` and `glab`. An item that
    /// closed or merged leaves the list. What's wrong with a tool goes in `tool_errors`, and
    /// leaves it when the tool works again.
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
        for item in Self::list(db).await? {
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
    /// # Errors
    /// When the update fails.
    pub async fn revised<C: ConnectionTrait>(
        mut self,
        db: &C,
        params: &Params,
    ) -> ModelResult<Model> {
        self.priority = Set(params.priority.clone());
        self.note = Set(params.note.trim().to_string());
        Ok(self.update(db).await?)
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
