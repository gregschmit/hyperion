pub use super::_entities::runs::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;

use super::schedules;

pub type Runs = Entity;

/// Where a run is. The table is both the queue and the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Ok,
    Failed,
}

impl Status {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
        }
    }
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
    /// The latest runs, newest first.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn recent(db: &DatabaseConnection, limit: u64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_desc(Column::Id)
            .limit(limit)
            .all(db)
            .await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such run.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// Whether run `id` is in progress.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn is_running(db: &DatabaseConnection, id: i64) -> ModelResult<bool> {
        let run = Entity::find_by_id(id).one(db).await?;
        Ok(run.is_some_and(|run| run.status == Status::Running.as_str()))
    }

    /// Queue a run of `schedule`, and give it back. If one already waits, that's the one.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn queue(db: &DatabaseConnection, schedule: &schedules::Model) -> ModelResult<Self> {
        let txn = db.begin().await?;
        let waiting = Entity::find()
            .filter(Column::Status.eq(Status::Queued.as_str()))
            .filter(Column::ScheduleId.eq(schedule.id))
            .one(&txn)
            .await?;
        if let Some(run) = waiting {
            return Ok(run);
        }
        let mut run = ActiveModel::new();
        run.schedule_id = Set(Some(schedule.id));
        run.copy(schedule);
        run.status = Set(Status::Queued.as_str().to_string());
        let run = run.insert(&txn).await?;
        txn.commit().await?;
        Ok(run)
    }

    /// The run that has waited the longest.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn next(db: &DatabaseConnection) -> ModelResult<Option<Self>> {
        Ok(Entity::find()
            .filter(Column::Status.eq(Status::Queued.as_str()))
            .order_by_asc(Column::Id)
            .one(db)
            .await?)
    }

    /// Fail the runs that are marked as running. Only the tick that holds the run lock calls
    /// this, before it starts a run, so whatever ran these is gone.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn fail_running(db: &DatabaseConnection, why: &str) -> ModelResult<()> {
        let running = Entity::find()
            .filter(Column::Status.eq(Status::Running.as_str()))
            .all(db)
            .await?;
        for run in running {
            let output = run.output.clone();
            run.into_active_model()
                .finished(db, Status::Failed, output, why.to_string())
                .await?;
        }
        Ok(())
    }
}

impl ActiveModel {
    /// What the history keeps of the run's schedule, which can change or go.
    fn copy(&mut self, schedule: &schedules::Model) {
        self.operation = Set(schedule.operation.clone());
        self.repo = Set(schedule.repo.clone());
        self.command = Set(schedule.command.clone());
    }

    /// The run starts, with this prompt. It takes its schedule as the schedule is now.
    ///
    /// # Errors
    /// When the update fails.
    pub async fn started(
        mut self,
        db: &DatabaseConnection,
        schedule: &schedules::Model,
        prompt: String,
    ) -> ModelResult<Model> {
        self.copy(schedule);
        self.status = Set(Status::Running.as_str().to_string());
        self.prompt = Set(prompt);
        self.started_at = Set(Some(chrono::Utc::now().into()));
        Ok(self.update(db).await?)
    }

    /// The run is over: `output` is what its command printed, and `error` is why m8r calls it
    /// failed, if it does.
    ///
    /// # Errors
    /// When the update fails.
    pub async fn finished(
        mut self,
        db: &DatabaseConnection,
        status: Status,
        output: String,
        error: String,
    ) -> ModelResult<Model> {
        self.status = Set(status.as_str().to_string());
        self.output = Set(output);
        self.error = Set(error);
        self.finished_at = Set(Some(chrono::Utc::now().into()));
        Ok(self.update(db).await?)
    }
}
