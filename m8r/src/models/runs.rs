pub use super::_entities::runs::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;

use super::configurations::Operation;

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

    /// Queue a run, unless one for the same operation and repo is already waiting.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn queue(
        db: &DatabaseConnection,
        operation: Operation,
        repo: &str,
    ) -> ModelResult<Option<Self>> {
        let txn = db.begin().await?;
        let waiting = Entity::find()
            .filter(Column::Status.eq(Status::Queued.as_str()))
            .filter(Column::Operation.eq(operation.as_str()))
            .filter(Column::Repo.eq(repo))
            .one(&txn)
            .await?;
        if waiting.is_some() {
            return Ok(None);
        }
        let mut run = ActiveModel::new();
        run.operation = Set(operation.as_str().to_string());
        run.repo = Set(repo.to_string());
        run.status = Set(Status::Queued.as_str().to_string());
        let run = run.insert(&txn).await?;
        txn.commit().await?;
        Ok(Some(run))
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
    /// The run starts, with this prompt.
    ///
    /// # Errors
    /// When the update fails.
    pub async fn started(mut self, db: &DatabaseConnection, prompt: String) -> ModelResult<Model> {
        self.status = Set(Status::Running.as_str().to_string());
        self.prompt = Set(prompt);
        self.started_at = Set(Some(chrono::Utc::now().into()));
        Ok(self.update(db).await?)
    }

    /// The run is over: `output` is what the agent printed, and `error` is why m8r calls it
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
