use std::collections::HashSet;

pub use super::_entities::schedules::{ActiveModel, Column, Entity, Model};
use hron::Schedule;
use jiff::Zoned;
use loco_rs::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};

use super::{errors, repos, runs, schedule_repos, watchlist_items::Category, Checked};
use crate::prompts;

pub type Schedules = Entity;

/// The units of an interval, as hron spells them.
const INTERVAL_UNITS: [&str; 8] = [
    "min", "mins", "minute", "minutes", "hour", "hours", "hr", "hrs",
];

/// Which of its repo's watchlist items a schedule's runs get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Items {
    None,
    All,
    Of(Category),
}

impl Items {
    pub const CHOICES: [Self; 4] = [
        Self::None,
        Self::All,
        Self::Of(Category::Issues),
        Self::Of(Category::Requests),
    ];

    /// As a schedule keeps it: blank for none, `all`, or a category's name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::All => "all",
            Self::Of(category) => category.as_str(),
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::CHOICES
            .into_iter()
            .find(|items| items.as_str() == text.trim())
    }
}

/// A list as text, one to a line: a schedule's times, as its form sends them and as it keeps
/// them, and the paths of its repos. The API may send an array. Not commas: an hron or cron
/// expression has its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Lines(String);

impl Lines {
    /// What's on the list.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        lines(&self.0)
    }
}

impl From<&str> for Lines {
    fn from(text: &str) -> Self {
        Self(text.to_string())
    }
}

impl<'de> Deserialize<'de> for Lines {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Sent {
            Text(String),
            List(Vec<String>),
        }
        Ok(match Sent::deserialize(deserializer)? {
            Sent::Text(text) => Self(text),
            Sent::List(list) => Self(list.join("\n")),
        })
    }
}

/// The lines of `text` that say something, with no space around them.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|line| !line.is_empty())
}

/// What a schedule's form and the API send.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Params {
    /// What the schedule is called: what it's for, in your words.
    pub name: String,
    /// Which of its repo's watchlist items a run gets: `all`, or a category's name. Blank: no
    /// items.
    pub category: String,
    /// How many items a run gets at most: the first ones, and the requested ones are first.
    /// 0: all of them.
    pub max_items: u32,
    /// The repos that the runs start in, one run in each: the paths of their clones, as m8r
    /// has them.
    pub repos: Lines,
    /// When it runs: hron expressions, or five-field cron expressions. It runs at each one's
    /// times.
    pub hron: Lines,
    pub command: String,
    pub prompt: String,
}

/// What changes on a schedule: only what's there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Changes {
    pub name: Option<String>,
    pub category: Option<String>,
    pub max_items: Option<u32>,
    pub repos: Option<Lines>,
    pub hron: Option<Lines>,
    pub command: Option<String>,
    pub prompt: Option<String>,
}

/// A schedule and its repos: what the pages and the API show.
pub type Listed = (Model, Vec<repos::Model>);

impl Params {
    /// A new schedule's form: the agent's command, every item, and the prompt that names
    /// m8r's parts.
    #[must_use]
    pub fn starter(agent: &str) -> Self {
        Self {
            category: Items::All.as_str().to_string(),
            command: agent.to_string(),
            prompt: prompts::STARTER.to_string(),
            ..Self::default()
        }
    }

    /// The schedule as it is, with `changes`.
    #[must_use]
    pub fn changed((schedule, repos): &Listed, changes: Changes) -> Self {
        let paths: Vec<&str> = repos.iter().map(|repo| repo.path.as_str()).collect();
        Self {
            name: changes.name.unwrap_or_else(|| schedule.name.clone()),
            category: changes
                .category
                .unwrap_or_else(|| schedule.category.clone()),
            max_items: changes.max_items.unwrap_or_else(|| schedule.max_items()),
            repos: changes
                .repos
                .unwrap_or_else(|| paths.join("\n").as_str().into()),
            hron: changes
                .hron
                .unwrap_or_else(|| schedule.hron.as_str().into()),
            command: changes.command.unwrap_or_else(|| schedule.command.clone()),
            prompt: changes.prompt.unwrap_or_else(|| schedule.prompt.clone()),
        }
    }

    /// Why these can't be saved, if they can't.
    fn problem(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("Give the schedule a name.".to_string());
        }
        if Items::parse(&self.category).is_none() {
            return Some(
                "A schedule's category is all, issues or pull/merge requests: the watchlist \
                 items of their repo that its runs get. Blank: no items."
                    .to_string(),
            );
        }
        if self.repos.iter().next().is_none() {
            return Some("Say which repos the schedule runs in.".to_string());
        }
        if self.command.trim().is_empty() {
            return Some("Say which command the schedule runs.".to_string());
        }
        let mut times = self.hron.iter().peekable();
        if times.peek().is_none() {
            return Some(
                "Give the schedule an hron: when it runs, such as every weekday at 06:00."
                    .to_string(),
            );
        }
        times.find_map(|time| parse(time).err())
    }

    /// The repos that these name, or why they can't be saved for schedule `id` (none: a new
    /// one). A repo is one that m8r has, and the prompt must compile with what a run would get
    /// now.
    async fn checked(
        &self,
        db: &DatabaseConnection,
        id: Option<i64>,
    ) -> ModelResult<Checked<Vec<repos::Model>>> {
        if let Some(problem) = self.problem() {
            return Ok(Err(problem));
        }
        let name = self.name.trim();
        let named = Entity::find().filter(Column::Name.eq(name)).one(db).await?;
        if named.is_some_and(|other| Some(other.id) != id) {
            return Ok(Err(format!("There's already a schedule \"{name}\".")));
        }
        // Each repo once, however its path is written: two runs mustn't work in one clone.
        let mut repos: Vec<repos::Model> = Vec::new();
        for path in self.repos.iter() {
            let Some(repo) = repos::Model::at(db, path).await? else {
                return Ok(Err(format!(
                    "\"{path}\" isn't one of m8r's repos. Add it to the repos first."
                )));
            };
            if repos.iter().all(|known| known.id != repo.id) {
                repos.push(repo);
            }
        }
        // By path, as a list has them.
        repos.sort_by(|one, other| one.path.cmp(&other.path));
        let context = self.context(db, repos.first()).await?;
        Ok(context.compile(&self.prompt).map(|_| repos))
    }

    /// What a prompt of these would get now, in `repo`: that repo's items, and a sample
    /// artifact directory. The pages show a prompt compiled with it.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn context(
        &self,
        db: &DatabaseConnection,
        repo: Option<&repos::Model>,
    ) -> ModelResult<prompts::Context> {
        let items = Items::parse(&self.category).unwrap_or(Items::None);
        let dir = prompts::SAMPLE_ARTIFACTS_DIR;
        prompts::Context::load(db, repo, items, self.max_items, dir).await
    }
}

/// Read one of a schedule's times: an hron expression such as `every weekday at 06:00`, or a
/// five-field cron expression such as `0 6 * * 1-5`, which hron turns into one of its own.
///
/// # Errors
/// A message that says what's wrong with it.
pub fn parse(text: &str) -> std::result::Result<Schedule, String> {
    let text = text.trim();
    // Cron starts with a number or a star, and hron never does.
    if text.starts_with(|first: char| first.is_ascii_digit() || first == '*') {
        return Schedule::from_cron(text)
            .map_err(|err| format!("m8r can't use that cron expression: {err}."));
    }
    Schedule::parse(&whole_day(text)).map_err(|err| err.display_rich())
}

/// hron wants a window on an interval: `every 12 hours from 00:00 to 23:59`. An interval
/// without one, such as `every 12 hours` or `every hour`, means the whole day.
fn whole_day(text: &str) -> String {
    let is = |word: Option<&&str>, wanted: &[&str]| {
        word.is_some_and(|word| wanted.contains(&word.to_ascii_lowercase().as_str()))
    };
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if !is(words.first(), &["every"]) {
        return text.to_string();
    }
    if is(words.get(1), &INTERVAL_UNITS) {
        words.insert(1, "1");
    }
    let counted = words.get(1).is_some_and(|word| word.parse::<u32>().is_ok());
    if counted && is(words.get(2), &INTERVAL_UNITS) && !is(words.get(3), &["from"]) {
        words.splice(3..3, ["from", "00:00", "to", "23:59"]);
    }
    words.join(" ")
}

/// A schedule that names no time zone runs on the clock of `at`, the server's. hron alone
/// would take UTC.
fn on_clock_of(schedule: Schedule, at: &Zoned) -> Schedule {
    if schedule.timezone().is_some() {
        return schedule;
    }
    let Some(zone) = at.time_zone().iana_name() else {
        return schedule;
    };
    let mut parts = schedule.to_parts();
    parts.timezone = Some(zone.to_string());
    Schedule::from_parts(parts).unwrap_or(schedule)
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
    /// Every schedule with its repos, by name.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection) -> ModelResult<Vec<Listed>> {
        let mut schedules = Entity::find()
            .find_with_related(repos::Entity)
            .all(db)
            .await?;
        schedules.sort_by(|(one, _), (other, _)| one.name.cmp(&other.name));
        for (_, repos) in &mut schedules {
            repos.sort_by(|one, other| one.path.cmp(&other.path));
        }
        Ok(schedules)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such schedule.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Listed> {
        let schedule = Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)?;
        let repos = schedule.repos(db).await?;
        Ok((schedule, repos))
    }

    /// The repos that the schedule's runs start in: one run in each, side by side.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn repos(&self, db: &DatabaseConnection) -> ModelResult<Vec<repos::Model>> {
        Ok(self
            .find_related(repos::Entity)
            .order_by_asc(repos::Column::Path)
            .all(db)
            .await?)
    }

    /// # Errors
    /// When a query fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Checked<Listed>> {
        let repos = match params.checked(db, None).await? {
            Ok(repos) => repos,
            Err(problem) => return Ok(Err(problem)),
        };
        let txn = db.begin().await?;
        let mut schedule = ActiveModel::new();
        schedule.set(params);
        let schedule = schedule.insert(&txn).await?;
        schedule.run_in(&txn, &repos).await?;
        txn.commit().await?;
        Ok(Ok((schedule, repos)))
    }

    /// Change the schedule: `params` are all that it has from you.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn revised(
        self,
        db: &DatabaseConnection,
        params: &Params,
    ) -> ModelResult<Checked<Listed>> {
        let repos = match params.checked(db, Some(self.id)).await? {
            Ok(repos) => repos,
            Err(problem) => return Ok(Err(problem)),
        };
        let txn = db.begin().await?;
        let mut schedule = self.into_active_model();
        schedule.set(params);
        let schedule = schedule.update(&txn).await?;
        schedule.run_in(&txn, &repos).await?;
        txn.commit().await?;
        // A repo that left the schedule leaves nothing behind: no run that waits to start
        // there, and no error that only a run there could clear.
        runs::Model::unqueue(db, schedule.id, &repos).await?;
        let paths = repos.iter().map(|repo| repo.path.as_str());
        errors::Model::clear_schedule(db, schedule.id, paths).await?;
        Ok(Ok((schedule, repos)))
    }

    /// Change schedule `id`: only what `changes` has.
    ///
    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such schedule, or when a query fails.
    pub async fn change(
        db: &DatabaseConnection,
        id: i64,
        changes: Changes,
    ) -> ModelResult<Checked<Listed>> {
        let listed = Self::find(db, id).await?;
        let params = Params::changed(&listed, changes);
        listed.0.revised(db, &params).await
    }

    /// Remove schedule `id`. Its runs stay in the history.
    ///
    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such schedule, or when a query fails.
    pub async fn remove(db: &DatabaseConnection, id: i64) -> ModelResult<()> {
        Self::find(db, id).await?.0.delete(db).await?;
        // No run of it can go well any more, so its errors are over.
        errors::Model::clear_schedule(db, id, []).await
    }

    /// Make `repos` the ones that the schedule runs in.
    async fn run_in<C: ConnectionTrait>(&self, db: &C, repos: &[repos::Model]) -> ModelResult<()> {
        schedule_repos::Entity::delete_many()
            .filter(schedule_repos::Column::ScheduleId.eq(self.id))
            .exec(db)
            .await?;
        for repo in repos {
            let row = schedule_repos::ActiveModel {
                schedule_id: Set(self.id),
                repo_id: Set(repo.id),
            };
            row.insert(db).await?;
        }
        Ok(())
    }

    /// Which of its repo's watchlist items a run of the schedule gets.
    #[must_use]
    pub fn items(&self) -> Items {
        Items::parse(&self.category).unwrap_or(Items::None)
    }

    /// How many items a run gets at most, as a count. 0: all of them.
    #[must_use]
    pub fn max_items(&self) -> u32 {
        u32::try_from(self.max_items).unwrap_or_default()
    }

    /// When the schedule runs: each of these is an hron or a cron expression.
    pub fn times(&self) -> impl Iterator<Item = &str> {
        lines(&self.hron)
    }

    /// Whether one of the schedule's times comes in the minute of `at`.
    #[must_use]
    pub fn is_due(&self, at: &Zoned) -> bool {
        self.times()
            .filter_map(|time| parse(time).ok())
            .any(|schedule| on_clock_of(schedule, at).matches(at))
    }

    /// The schedule's next time after `at`, on the clock of `at`.
    #[must_use]
    pub fn next(&self, at: &Zoned) -> Option<Zoned> {
        let next = self
            .times()
            .filter_map(|time| on_clock_of(parse(time).ok()?, at).next_from(at))
            .min_by_key(Zoned::timestamp)?;
        Some(next.with_time_zone(at.time_zone().clone()))
    }
}

impl ActiveModel {
    fn set(&mut self, params: &Params) {
        self.name = Set(params.name.trim().to_string());
        self.category = Set(params.category.trim().to_string());
        self.max_items = Set(i64::from(params.max_items));
        // As hron writes them, so the pages show how m8r read them.
        let mut seen = HashSet::new();
        let times: Vec<String> = params
            .hron
            .iter()
            .map(|time| parse(time).map_or_else(|_| time.to_string(), |read| read.to_string()))
            .filter(|time| seen.insert(time.clone()))
            .collect();
        self.hron = Set(times.join("\n"));
        self.command = Set(params.command.trim().to_string());
        self.prompt = Set(params.prompt.clone());
    }
}
