//! The repos that m8r works in. A repo is a clone in the home of m8r's user, and the project
//! on its forge. A schedule runs in repos, and a watchlist item is in the repo whose project
//! its URL names.

use std::path::{Path, PathBuf};

pub use super::_entities::repos::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{errors, schedule_repos, watchlist_items, Checked};
use crate::tools;

pub type Repos = Entity;

/// What the repos' forms and the API send.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Params {
    /// The clone: a path from the home of m8r's user, such as `~/git/group/project`, or a
    /// full path.
    pub path: String,
    /// The project on its forge, such as `https://git.example.com/group/project`. Blank: m8r
    /// reads it from the clone's `origin`.
    pub url: String,
}

/// What changes on a repo: only what's there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Changes {
    pub path: Option<String>,
    pub url: Option<String>,
}

/// The directory of a clone, as the user wrote its path: from the home of the user that m8r
/// runs as, unless it's a full path.
#[must_use]
pub fn dir_of(path: &str) -> PathBuf {
    match std::env::home_dir() {
        Some(home) => from_home(path, &home),
        None => PathBuf::from(path),
    }
}

/// `path` from `home`: `~/git/rxg` and `git/rxg` are both in it. A full path stays.
#[must_use]
pub fn from_home(path: &str, home: &Path) -> PathBuf {
    let path = path.trim();
    let inside = path
        .strip_prefix("~/")
        .or_else(|| (path == "~").then_some(""))
        .unwrap_or(path);
    // Joining a full path gives that path.
    home.join(inside)
}

/// `dir` as the system knows it, so that two ways to write one directory are the same. A
/// directory that isn't there stays as it's written.
fn real(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
}

/// A project's URL as m8r keeps it, `https://<host>/<group>/<project>`. `text` is that URL
/// as a browser shows it, or one of the clone's remote: `https://host/group/project.git`,
/// `git@host:group/project.git` or `ssh://git@host:2222/group/project.git`.
#[must_use]
pub fn project_url(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches('/');
    let text = text.strip_suffix(".git").unwrap_or(text);
    let (host, path) = match text.split_once("://") {
        // A browser's port is part of the host. The port of another protocol isn't.
        Some(("http" | "https", rest)) => rest.split_once('/')?,
        Some((_, rest)) => {
            let (host, path) = rest.split_once('/')?;
            (host.split(':').next()?, path)
        }
        // `git@host:group/project`, or with no colon, `host/group/project`.
        None => text.split_once(':').or_else(|| text.split_once('/'))?,
    };
    let host = host.rsplit('@').next()?.to_ascii_lowercase();
    let plain = |text: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    let parts: Vec<&str> = path.split('/').collect();
    if !host.split(':').all(plain) || parts.len() < 2 || !parts.iter().all(|part| plain(part)) {
        return None;
    }
    Some(format!("https://{host}/{}", parts.join("/")))
}

impl Params {
    /// The repo as it is, with `changes`.
    #[must_use]
    pub fn changed(repo: &Model, changes: Changes) -> Self {
        Self {
            path: changes.path.unwrap_or_else(|| repo.path.clone()),
            url: changes.url.unwrap_or_else(|| repo.url.clone()),
        }
    }

    /// The path and the project's URL to save for repo `id` (none: a new one), or why these
    /// can't be saved. With no URL here, the clone's `origin` says it, if it can.
    async fn checked(
        &self,
        db: &DatabaseConnection,
        shell: &str,
        id: Option<i64>,
    ) -> ModelResult<Checked<(String, String)>> {
        let path = self.path.trim();
        if path.is_empty() {
            return Ok(Err(
                "Say where the clone is: a path from the home of m8r's user, such as \
                 ~/git/group/project."
                    .to_string(),
            ));
        }
        let dir = dir_of(path);
        if !dir.is_dir() {
            return Ok(Err(format!(
                "A repo is a clone in the home of m8r's user, such as ~/git/group/project, and \
                 \"{path}\" isn't a directory there."
            )));
        }
        let others: Vec<Model> = Model::list(db)
            .await?
            .into_iter()
            .filter(|other| Some(other.id) != id)
            .collect();
        if let Some(other) = others.iter().find(|other| real(&other.dir()) == real(&dir)) {
            return Ok(Err(format!("m8r already has that clone: {}.", other.path)));
        }
        let url = if self.url.trim().is_empty() {
            origin(shell, &dir).await.unwrap_or_default()
        } else {
            let Some(url) = project_url(&self.url) else {
                return Ok(Err(
                    "That isn't the URL of a project on GitHub or GitLab, such as \
                     https://git.example.com/group/project."
                        .to_string(),
                ));
            };
            url
        };
        if let Some(other) = others.iter().find(|other| other.is(&url)) {
            return Ok(Err(format!(
                "{url} is the project of the repo {}, and a project has one repo.",
                other.path
            )));
        }
        Ok(Ok((path.to_string(), url)))
    }
}

/// The project that the clone in `dir` came from: its `origin`, if it has one that m8r can
/// read.
async fn origin(shell: &str, dir: &Path) -> Option<String> {
    // git would look in the directories above too, and answer for a clone that holds this one.
    if !dir.join(".git").exists() {
        return None;
    }
    let dir = dir.to_string_lossy();
    let argv = ["git", "-C", dir.as_ref(), "remote", "get-url", "origin"];
    let output = tools::run(shell, &argv.map(str::to_string), tools::ANSWER_LIMIT)
        .await
        .ok()?;
    if !output.ok() {
        return None;
    }
    // A login shell may print something of its own first.
    project_url(
        output
            .stdout
            .lines()
            .rfind(|line| !line.trim().is_empty())?,
    )
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
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find().order_by_asc(Column::Path).all(db).await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such repo.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// The repo whose clone is at `path`, however the path is written.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn at(db: &DatabaseConnection, path: &str) -> ModelResult<Option<Self>> {
        let repos = Self::list(db).await?;
        Ok(repos.into_iter().find(|repo| repo.is_at(path)))
    }

    /// The repo of the project at `url`, a URL as `project_url` gives it.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn of_project(db: &DatabaseConnection, url: &str) -> ModelResult<Option<Self>> {
        let repos = Self::list(db).await?;
        Ok(repos.into_iter().find(|repo| repo.is(url)))
    }

    /// Add a repo. `shell` starts `git`, which reads the project from the clone when `params`
    /// name none.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn create(
        db: &DatabaseConnection,
        shell: &str,
        params: &Params,
    ) -> ModelResult<Checked<Self>> {
        let (path, url) = match params.checked(db, shell, None).await? {
            Ok(checked) => checked,
            Err(problem) => return Ok(Err(problem)),
        };
        let mut repo = ActiveModel::new();
        repo.path = Set(path);
        repo.url = Set(url);
        Ok(Ok(repo.insert(db).await?))
    }

    /// Change the repo: `params` are all that it has from you.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn revised(
        self,
        db: &DatabaseConnection,
        shell: &str,
        params: &Params,
    ) -> ModelResult<Checked<Self>> {
        let (path, url) = match params.checked(db, shell, Some(self.id)).await? {
            Ok(checked) => checked,
            Err(problem) => return Ok(Err(problem)),
        };
        // An error names the clone's path, so one about the path from before has nothing
        // left to clear it.
        if path != self.path {
            errors::Model::clear_repo(db, &self.path).await?;
        }
        let mut repo = self.into_active_model();
        repo.path = Set(path);
        repo.url = Set(url);
        Ok(Ok(repo.update(db).await?))
    }

    /// Change repo `id`: only what `changes` has.
    ///
    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such repo, or when a query fails.
    pub async fn change(
        db: &DatabaseConnection,
        shell: &str,
        id: i64,
        changes: Changes,
    ) -> ModelResult<Checked<Self>> {
        let repo = Self::find(db, id).await?;
        let params = Params::changed(&repo, changes);
        repo.revised(db, shell, &params).await
    }

    /// How many watchlist items are in the repo, and how many schedules run in it.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn uses(&self, db: &DatabaseConnection) -> ModelResult<(u64, u64)> {
        let items = watchlist_items::Entity::find()
            .filter(watchlist_items::Column::RepoId.eq(self.id))
            .count(db)
            .await?;
        let schedules = schedule_repos::Entity::find()
            .filter(schedule_repos::Column::RepoId.eq(self.id))
            .count(db)
            .await?;
        Ok((items, schedules))
    }

    /// Remove the repo, unless a watchlist item is in it or a schedule runs in it. Its runs
    /// stay in the history.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn removed(self, db: &DatabaseConnection) -> ModelResult<Checked<()>> {
        let (items, schedules) = self.uses(db).await?;
        if items + schedules > 0 {
            return Ok(Err(format!(
                "\"{}\" is in use: by {items} watchlist items and {schedules} schedules. Take \
                 them off it first.",
                self.path
            )));
        }
        let path = self.path.clone();
        self.delete(db).await?;
        errors::Model::clear_repo(db, &path).await?;
        Ok(Ok(()))
    }

    /// Where the clone is.
    #[must_use]
    pub fn dir(&self) -> PathBuf {
        dir_of(&self.path)
    }

    /// Whether the repo's clone is at `path`, however the path is written.
    #[must_use]
    pub fn is_at(&self, path: &str) -> bool {
        real(&self.dir()) == real(&dir_of(path))
    }

    /// Whether the repo's project is the one at `url`. A forge reads its paths without case.
    #[must_use]
    pub fn is(&self, url: &str) -> bool {
        !url.is_empty() && self.url.eq_ignore_ascii_case(url)
    }

    /// Look at every repo, as a tick does. A clone that isn't there is an error until it is.
    /// A repo with no project gets the one that its clone's `origin` names, unless another
    /// repo has it: that's an error until one of the two goes.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn check(db: &DatabaseConnection, shell: &str) -> ModelResult<()> {
        for repo in Self::list(db).await? {
            let (subject, dir) = (errors::repo(&repo.path), repo.dir());
            if !dir.is_dir() {
                let message = format!(
                    "The clone isn't there: {} isn't a directory. A run there fails, until you \
                     clone the repo there or change its path.",
                    dir.display()
                );
                errors::Model::raise(db, &subject, &message, None).await?;
                continue;
            }
            let found = if repo.url.is_empty() {
                origin(shell, &dir).await
            } else {
                None
            };
            let Some(url) = found else {
                errors::Model::clear(db, &subject).await?;
                continue;
            };
            if let Some(other) = Self::of_project(db, &url).await? {
                let message = format!(
                    "Its origin is {url}, which is the project of the repo {}. A project has \
                     one repo, so remove one of the two.",
                    other.path
                );
                errors::Model::raise(db, &subject, &message, None).await?;
                continue;
            }
            let mut known = repo.into_active_model();
            known.url = Set(url);
            known.update(db).await?;
            errors::Model::clear(db, &subject).await?;
        }
        Ok(())
    }
}
