//! What's wrong now, and what was. Nobody clears an error by hand: whatever raises one
//! clears it too, when its subject is well again.

pub use super::_entities::errors::{ActiveModel, Column, Entity, Model};
use chrono::{Duration, Utc};
use loco_rs::prelude::*;
use sea_orm::SqlErr;

use super::schedules;

pub type Errors = Entity;

/// There's no schedule. `check_setup` raises it and clears it.
pub const SCHEDULES: &str = "schedules";
/// The scheduler doesn't tick. A page raises it, and a tick clears it.
pub const SCHEDULER: &str = "scheduler";
/// A run can't reach m8r's API. A tick raises and clears it.
pub const API: &str = "API";

/// How long the list keeps an error that was cleared.
const KEPT: Duration = Duration::days(30);

/// The subject of schedule `id` in `repo`, the path of one of its repos: a run there that
/// fails raises its error, and one that goes well clears it.
#[must_use]
pub fn schedule(id: i64, repo: &str) -> String {
    format!("{}{repo}", schedule_start(id))
}

/// How the subjects of schedule `id` start, whichever repo they're about.
fn schedule_start(id: i64) -> String {
    format!("schedule {id} in ")
}

/// The subject of the repo whose clone is at `path`. A tick raises its error and clears it
/// (see `repos::Model::check`).
#[must_use]
pub fn repo(path: &str) -> String {
    format!("repo {path}")
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
    /// The errors that aren't cleared, the oldest first. Every page shows them.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn open(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::ClearedAt.is_null())
            .order_by_asc(Column::Id)
            .all(db)
            .await?)
    }

    /// The open errors, and then the latest `limit` that were cleared.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn list(db: &DatabaseConnection, limit: u64) -> ModelResult<Vec<Self>> {
        let mut errors = Self::open(db).await?;
        let cleared = Entity::find()
            .filter(Column::ClearedAt.is_not_null())
            .order_by_desc(Column::ClearedAt)
            .order_by_desc(Column::Id)
            .limit(limit)
            .all(db)
            .await?;
        errors.extend(cleared);
        Ok(errors)
    }

    /// Something is wrong with `subject`. Its one open error says so, with the latest
    /// `message` and the run that failed, until `clear`.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn raise(
        db: &DatabaseConnection,
        subject: &str,
        message: &str,
        run_id: Option<i64>,
    ) -> ModelResult<()> {
        let open = Entity::find()
            .filter(Column::Subject.eq(subject))
            .filter(Column::ClearedAt.is_null())
            .one(db)
            .await?;
        if let Some(error) = open {
            if error.message != message || error.run_id != run_id {
                let mut error = error.into_active_model();
                error.message = Set(message.to_string());
                error.run_id = Set(run_id);
                error.update(db).await?;
            }
            return Ok(());
        }
        let mut error = ActiveModel::new();
        error.subject = Set(subject.to_string());
        error.message = Set(message.to_string());
        error.run_id = Set(run_id);
        match error.insert(db).await {
            Ok(_) => Ok(()),
            // Another process raised it in the same moment: a subject has one open error.
            Err(err) if matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }

    /// `subject` is well: its open error, if it has one, is over. The list keeps it.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn clear(db: &DatabaseConnection, subject: &str) -> ModelResult<()> {
        Entity::update_many()
            .col_expr(Column::ClearedAt, Expr::value(Utc::now()))
            .filter(Column::Subject.eq(subject))
            .filter(Column::ClearedAt.is_null())
            .exec(db)
            .await?;
        Ok(())
    }

    /// Schedule `id` has `repos` now, or with none, it's gone. Its open errors about other
    /// repos are over: no run can clear them any more.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn clear_schedule<'a>(
        db: &DatabaseConnection,
        id: i64,
        repos: impl IntoIterator<Item = &'a str>,
    ) -> ModelResult<()> {
        let kept: Vec<String> = repos.into_iter().map(|repo| schedule(id, repo)).collect();
        Entity::update_many()
            .col_expr(Column::ClearedAt, Expr::value(Utc::now()))
            .filter(Column::Subject.starts_with(schedule_start(id)))
            .filter(Column::Subject.is_not_in(kept))
            .filter(Column::ClearedAt.is_null())
            .exec(db)
            .await?;
        Ok(())
    }

    /// The repo at `path` is gone, or its clone is somewhere else now. The open errors that
    /// name the path are over: the repo's own, and the ones of schedules in it.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn clear_repo(db: &DatabaseConnection, path: &str) -> ModelResult<()> {
        let (own, end) = (repo(path), format!(" in {path}"));
        // Not LIKE: a path can hold its wildcards.
        for error in Self::open(db).await? {
            let subject = &error.subject;
            if *subject == own || (subject.starts_with("schedule ") && subject.ends_with(&end)) {
                Self::clear(db, subject).await?;
            }
        }
        Ok(())
    }

    /// Raise `subject`'s error if there's a `problem`, and clear it if there's none. For a
    /// check that knows both.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn set(
        db: &DatabaseConnection,
        subject: &str,
        problem: Option<&str>,
    ) -> ModelResult<()> {
        match problem {
            Some(message) => Self::raise(db, subject, message, None).await,
            None => Self::clear(db, subject).await,
        }
    }

    /// m8r runs nothing without a schedule, which is an error while it's missing. Every tick
    /// asks, and so does every page, so that the error goes as soon as you look after you put
    /// it right.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn check_setup(db: &DatabaseConnection) -> ModelResult<()> {
        let scheduled = schedules::Entity::find().count(db).await? > 0;
        let problem = "There's no schedule, so nothing runs. Add one.";
        Self::set(db, SCHEDULES, (!scheduled).then_some(problem)).await
    }

    /// Drop the errors that were cleared long ago.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn forget(db: &DatabaseConnection) -> ModelResult<()> {
        Entity::delete_many()
            .filter(Column::ClearedAt.lt(Utc::now() - KEPT))
            .exec(db)
            .await?;
        Ok(())
    }
}
