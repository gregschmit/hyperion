pub use super::_entities::configurations::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::Checked;
use crate::prompts;

pub type Configurations = Entity;

/// What a configuration's form and the API send.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Params {
    pub name: String,
    pub system_message: String,
}

/// What changes on a configuration: only what's there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Changes {
    pub name: Option<String>,
    pub system_message: Option<String>,
}

impl Params {
    /// A new configuration's form: m8r's own system message.
    #[must_use]
    pub fn starter() -> Self {
        Self {
            system_message: prompts::DEFAULT_SYSTEM_MESSAGE.to_string(),
            ..Self::default()
        }
    }

    /// The configuration as it is, with `changes`.
    #[must_use]
    pub fn changed(configuration: &Model, changes: Changes) -> Self {
        Self {
            name: changes.name.unwrap_or_else(|| configuration.name.clone()),
            system_message: changes
                .system_message
                .unwrap_or_else(|| configuration.system_message.clone()),
        }
    }

    /// The system message as a run would get it now. Which watchlist items a run gets is its
    /// schedule's to say, so here there are none.
    ///
    /// # Errors
    /// When a query fails. Inside: Tera's message, when the template doesn't compile.
    pub async fn compiled(&self, db: &DatabaseConnection) -> ModelResult<Checked<String>> {
        let context = prompts::Context {
            system_message: self.system_message.clone(),
            ..prompts::Context::load(db, None, prompts::SAMPLE_ARTIFACTS_DIR).await?
        };
        Ok(context.compile("{{ system_message }}"))
    }

    /// Why these can't be saved, if they can't: no name, or a system message that doesn't
    /// compile.
    async fn problem(&self, db: &DatabaseConnection) -> ModelResult<Option<String>> {
        if self.name.trim().is_empty() {
            return Ok(Some("Give the configuration a name.".to_string()));
        }
        Ok(self.compiled(db).await?.err())
    }
}

impl From<&Model> for Params {
    fn from(configuration: &Model) -> Self {
        Self {
            name: configuration.name.clone(),
            system_message: configuration.system_message.clone(),
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
    /// Every configuration, the active one first.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_desc(Column::Active)
            .order_by_asc(Column::Name)
            .all(db)
            .await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such configuration.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// The configuration whose system message the runs get. With none, nothing runs.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn active(db: &DatabaseConnection) -> ModelResult<Option<Self>> {
        Ok(Entity::find()
            .filter(Column::Active.eq(true))
            .one(db)
            .await?)
    }

    /// # Errors
    /// When a query fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Checked<Self>> {
        if let Some(problem) = params.problem(db).await? {
            return Ok(Err(problem));
        }
        let mut configuration = ActiveModel::new();
        configuration.set(params);
        Ok(Ok(configuration.insert(db).await?))
    }

    /// Change configuration `id`: only what `changes` has.
    ///
    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such configuration, or when a query fails.
    pub async fn change(
        db: &DatabaseConnection,
        id: i64,
        changes: Changes,
    ) -> ModelResult<Checked<Self>> {
        let configuration = Self::find(db, id).await?;
        let params = Params::changed(&configuration, changes);
        configuration.revised(db, &params).await
    }

    /// Change the configuration: `params` are all that it has from you.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn revised(
        self,
        db: &DatabaseConnection,
        params: &Params,
    ) -> ModelResult<Checked<Self>> {
        if let Some(problem) = params.problem(db).await? {
            return Ok(Err(problem));
        }
        let mut configuration = self.into_active_model();
        configuration.set(params);
        Ok(Ok(configuration.update(db).await?))
    }
}

impl ActiveModel {
    fn set(&mut self, params: &Params) {
        self.name = Set(params.name.trim().to_string());
        self.system_message = Set(params.system_message.clone());
    }

    /// Make this the one active configuration.
    ///
    /// # Errors
    /// When an update fails.
    pub async fn activated(mut self, db: &DatabaseConnection) -> ModelResult<Model> {
        let txn = db.begin().await?;
        Entity::update_many()
            .col_expr(Column::Active, Expr::value(false))
            .filter(Column::Active.eq(true))
            .exec(&txn)
            .await?;
        self.active = Set(true);
        let configuration = self.update(&txn).await?;
        txn.commit().await?;
        Ok(configuration)
    }

    /// # Errors
    /// When the update fails.
    pub async fn deactivated(mut self, db: &DatabaseConnection) -> ModelResult<Model> {
        self.active = Set(false);
        Ok(self.update(db).await?)
    }
}
