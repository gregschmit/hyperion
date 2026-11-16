use std::path::Path;

pub use super::_entities::schedules::{ActiveModel, Column, Entity, Model};
use hron::Schedule;
use jiff::Zoned;
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{categories, Checked};
use crate::prompts;

pub type Schedules = Entity;

/// The units of an interval, as hron spells them.
const INTERVAL_UNITS: [&str; 8] = [
    "min", "mins", "minute", "minutes", "hour", "hours", "hr", "hrs",
];

/// What a schedule is for. It names the schedule's runs, and it gives a new schedule its
/// command. Other is for anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Triage,
    Review,
    Develop,
    Other,
}

impl Operation {
    pub const ALL: [Self; 4] = [Self::Triage, Self::Review, Self::Develop, Self::Other];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Review => "review",
            Self::Develop => "develop",
            Self::Other => "other",
        }
    }

    /// The command that a new schedule for this operation starts with: the agent's, which
    /// reads the prompt. Other has none.
    #[must_use]
    pub const fn default_command(self, agent: &str) -> &str {
        match self {
            Self::Other => "",
            Self::Triage | Self::Review | Self::Develop => agent,
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.as_str() == name)
    }
}

/// What a schedule's form and the API send.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Params {
    pub operation: String,
    /// The name of the category whose watchlist items the runs get. Blank: no items.
    pub category: String,
    pub repo: String,
    /// When it runs: an hron expression, or a five-field cron expression.
    pub hron: String,
    pub command: String,
    pub prompt: String,
}

/// What changes on a schedule: only what's there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Changes {
    pub operation: Option<String>,
    pub category: Option<String>,
    pub repo: Option<String>,
    pub hron: Option<String>,
    pub command: Option<String>,
    pub prompt: Option<String>,
}

/// A schedule and its category, if it has one: what the pages and the API show.
pub type Listed = (Model, Option<categories::Model>);

impl Params {
    /// A new schedule's form: the first operation with its command, and the prompt that names
    /// m8r's parts.
    #[must_use]
    pub fn starter(agent: &str) -> Self {
        let operation = Operation::Triage;
        Self {
            operation: operation.as_str().to_string(),
            command: operation.default_command(agent).to_string(),
            prompt: prompts::STARTER.to_string(),
            ..Self::default()
        }
    }

    /// The schedule as it is, with `changes`.
    #[must_use]
    pub fn changed((schedule, category): &Listed, changes: Changes) -> Self {
        let category = category
            .as_ref()
            .map(|it| it.name.clone())
            .unwrap_or_default();
        Self {
            operation: changes
                .operation
                .unwrap_or_else(|| schedule.operation.clone()),
            category: changes.category.unwrap_or(category),
            repo: changes.repo.unwrap_or_else(|| schedule.repo.clone()),
            hron: changes.hron.unwrap_or_else(|| schedule.hron.clone()),
            command: changes.command.unwrap_or_else(|| schedule.command.clone()),
            prompt: changes.prompt.unwrap_or_else(|| schedule.prompt.clone()),
        }
    }

    /// Why these can't be saved, if they can't.
    fn problem(&self) -> Option<String> {
        if Operation::parse(&self.operation).is_none() {
            return Some("The operation is triage, review, develop or other.".to_string());
        }
        let repo = Path::new(self.repo.trim());
        if !repo.is_absolute() || !repo.is_dir() {
            return Some(format!(
                "The repo must be the full path of a clone that m8r's user can see, and \"{}\" \
                 isn't a directory.",
                self.repo.trim()
            ));
        }
        if self.command.trim().is_empty() {
            return Some("Say which command the schedule runs.".to_string());
        }
        parse(&self.hron).err()
    }

    /// The category that these name, if they name one, or why they can't be saved. The prompt
    /// must compile with what a run would get now.
    async fn checked(
        &self,
        db: &DatabaseConnection,
    ) -> ModelResult<Checked<Option<categories::Model>>> {
        if let Some(problem) = self.problem() {
            return Ok(Err(problem));
        }
        let category = if self.category.trim().is_empty() {
            None
        } else {
            match categories::Model::named(db, &self.category).await? {
                Ok(category) => Some(category),
                Err(problem) => return Ok(Err(problem)),
            }
        };
        let context =
            prompts::Context::load(db, category.as_ref(), prompts::SAMPLE_ARTIFACTS_DIR).await?;
        Ok(context.compile(&self.prompt).map(|_| category))
    }
}

/// Read when a schedule runs: an hron expression such as `every weekday at 06:00`, or a
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
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection) -> ModelResult<Vec<Listed>> {
        Ok(Entity::find()
            .find_also_related(categories::Entity)
            .order_by_asc(Column::Repo)
            .order_by_asc(Column::Operation)
            .order_by_asc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such schedule.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Listed> {
        Entity::find_by_id(id)
            .find_also_related(categories::Entity)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// # Errors
    /// When a query fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Checked<Listed>> {
        let category = match params.checked(db).await? {
            Ok(category) => category,
            Err(problem) => return Ok(Err(problem)),
        };
        let mut schedule = ActiveModel::new();
        schedule.set(params, category.as_ref());
        Ok(Ok((schedule.insert(db).await?, category)))
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
        let category = match params.checked(db).await? {
            Ok(category) => category,
            Err(problem) => return Ok(Err(problem)),
        };
        let mut schedule = self.into_active_model();
        schedule.set(params, category.as_ref());
        Ok(Ok((schedule.update(db).await?, category)))
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
        Ok(())
    }

    /// Whether the schedule's time comes in the minute of `at`.
    #[must_use]
    pub fn is_due(&self, at: &Zoned) -> bool {
        parse(&self.hron).is_ok_and(|schedule| on_clock_of(schedule, at).matches(at))
    }

    /// The schedule's next time after `at`, on the clock of `at`.
    #[must_use]
    pub fn next(&self, at: &Zoned) -> Option<Zoned> {
        let schedule = on_clock_of(parse(&self.hron).ok()?, at);
        let next = schedule.next_from(at)?;
        Some(next.with_time_zone(at.time_zone().clone()))
    }
}

impl ActiveModel {
    fn set(&mut self, params: &Params, category: Option<&categories::Model>) {
        self.operation = Set(params.operation.clone());
        self.category_id = Set(category.map(|it| it.id));
        self.repo = Set(params.repo.trim().to_string());
        // As hron writes it, so the pages show how m8r read it.
        self.hron = Set(parse(&params.hron).map_or_else(
            |_| params.hron.trim().to_string(),
            |schedule| schedule.to_string(),
        ));
        self.command = Set(params.command.trim().to_string());
        self.prompt = Set(params.prompt.clone());
    }
}
