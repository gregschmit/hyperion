use std::{path::Path, str::FromStr};

pub use super::_entities::schedules::{ActiveModel, Column, Entity, Model};
use chrono::{DateTime, Local, Timelike};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::configurations::Operation;

pub type Schedules = Entity;

/// What a schedule's form sends.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Params {
    pub operation: String,
    pub repo: String,
    pub cron: String,
}

impl Params {
    /// Why these can't be saved, if they can't.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if Operation::parse(&self.operation).is_none() {
            return Some("Choose triage, review or develop.".to_string());
        }
        let repo = Path::new(self.repo.trim());
        if !repo.is_absolute() || !repo.is_dir() {
            return Some(format!(
                "The repo must be the full path of a clone that m8r's user can see, and \"{}\" \
                 isn't a directory.",
                self.repo.trim()
            ));
        }
        parse_cron(&self.cron).err()
    }
}

/// Read a five-field cron expression, such as `0 6 * * 1-5`.
///
/// # Errors
/// A message that says what's wrong with it.
pub fn parse_cron(expression: &str) -> std::result::Result<croner::Cron, String> {
    let expression = expression.trim();
    if expression.split_whitespace().count() != 5 {
        return Err(
            "The cron expression needs five fields: minute, hour, day of the month, month, and \
             day of the week. For example, 0 6 * * 1-5."
                .to_string(),
        );
    }
    croner::Cron::from_str(expression)
        .map_err(|err| format!("The cron expression doesn't parse: {err}."))
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
        Ok(Entity::find()
            .order_by_asc(Column::Repo)
            .order_by_asc(Column::Operation)
            .order_by_asc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such schedule.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// # Errors
    /// When the insert fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Self> {
        let mut schedule = ActiveModel::new();
        schedule.set(params);
        Ok(schedule.insert(db).await?)
    }

    /// Whether the schedule's time comes in the minute of `at`.
    #[must_use]
    pub fn is_due(&self, at: &DateTime<Local>) -> bool {
        // A five-field expression matches second 0 only, and a tick starts a little later.
        let Some(minute) = at.with_second(0).and_then(|at| at.with_nanosecond(0)) else {
            return false;
        };
        parse_cron(&self.cron).is_ok_and(|cron| cron.is_time_matching(&minute).unwrap_or(false))
    }

    /// The schedule's next time after `at`.
    #[must_use]
    pub fn next(&self, at: &DateTime<Local>) -> Option<DateTime<Local>> {
        parse_cron(&self.cron)
            .ok()?
            .find_next_occurrence(at, false)
            .ok()
    }
}

impl ActiveModel {
    fn set(&mut self, params: &Params) {
        self.operation = Set(params.operation.clone());
        self.repo = Set(params.repo.trim().to_string());
        self.cron = Set(params.cron.split_whitespace().collect::<Vec<_>>().join(" "));
    }

    /// # Errors
    /// When the update fails.
    pub async fn revised(mut self, db: &DatabaseConnection, params: &Params) -> ModelResult<Model> {
        self.set(params);
        Ok(self.update(db).await?)
    }
}
