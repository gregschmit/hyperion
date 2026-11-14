pub use super::_entities::configurations::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::items;

pub type Configurations = Entity;

/// What a new configuration's system message starts as.
pub const DEFAULT_SYSTEM_MESSAGE: &str = "\
You are running in m8r, a maintainer automation service. Nobody is here to answer questions, so
decide for yourself.

These issues and PRs/MRs have priority, and high means urgent. Take the ones in this repository
first, and keep following each one until it closes:
{% for item in items %}- {{ item.priority }}: {{ item.url }} {{ item.note }}
{% else %}(none)
{% endfor %}
When you finish, report what you made as artifacts. For each one, make a directory in
{{ artifacts_dir }}, named for what it is. Put the artifact's files in it. If the artifact is
already online (a comment you posted, a PR/MR), put in one text file that holds only its link.
";

/// The artifact directory that a prompt is compiled with when no run is at hand.
pub const SAMPLE_ARTIFACTS_DIR: &str = "/var/lib/m8r/artifacts/1";

/// What a run does. Each one has a prompt of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Triage,
    Review,
    Develop,
}

impl Operation {
    pub const ALL: [Self; 3] = [Self::Triage, Self::Review, Self::Develop];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Review => "review",
            Self::Develop => "develop",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.as_str() == name)
    }
}

/// What a configuration's form sends.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Params {
    pub name: String,
    pub system_message: String,
    pub triage_prompt: String,
    pub review_prompt: String,
    pub develop_prompt: String,
}

impl Params {
    /// A new configuration's form: m8r's own system message, and no prompts yet.
    #[must_use]
    pub fn starter() -> Self {
        Self {
            system_message: DEFAULT_SYSTEM_MESSAGE.to_string(),
            ..Self::default()
        }
    }

    /// Why these can't be saved, if they can't: no name, or a template that doesn't compile.
    #[must_use]
    pub fn problem(&self, items: &[items::Model]) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("Give the configuration a name.".to_string());
        }
        Operation::ALL
            .into_iter()
            .find_map(|operation| self.compile(operation, items, SAMPLE_ARTIFACTS_DIR).err())
    }

    fn prompt(&self, operation: Operation) -> &str {
        match operation {
            Operation::Triage => &self.triage_prompt,
            Operation::Review => &self.review_prompt,
            Operation::Develop => &self.develop_prompt,
        }
    }

    /// Compile the prompt for `operation`: the system message first, then the prompt, which
    /// says where the system message goes.
    ///
    /// # Errors
    /// Tera's message, when a template doesn't compile.
    pub fn compile(
        &self,
        operation: Operation,
        items: &[items::Model],
        artifacts_dir: &str,
    ) -> std::result::Result<String, String> {
        // Not HTML: nothing in a prompt is escaped.
        let autoescape = false;
        let mut context = tera::Context::new();
        let items: Vec<_> = items
            .iter()
            .map(|item| {
                serde_json::json!({
                    "url": item.url,
                    "priority": item.priority,
                    "note": item.note,
                })
            })
            .collect();
        context.insert("items", &items);
        context.insert("artifacts_dir", artifacts_dir);
        let system_message = tera::Tera::one_off(&self.system_message, &context, autoescape)
            .map_err(|err| format!("The system message doesn't compile.\n{err}"))?;
        context.insert("system_message", &system_message);
        tera::Tera::one_off(self.prompt(operation), &context, autoescape)
            .map_err(|err| format!("The {} prompt doesn't compile.\n{err}", operation.as_str()))
    }
}

impl From<&Model> for Params {
    fn from(configuration: &Model) -> Self {
        Self {
            name: configuration.name.clone(),
            system_message: configuration.system_message.clone(),
            triage_prompt: configuration.triage_prompt.clone(),
            review_prompt: configuration.review_prompt.clone(),
            develop_prompt: configuration.develop_prompt.clone(),
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

    /// The configuration whose prompts the runs use. With none, nothing runs.
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
    /// When the insert fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Self> {
        let mut configuration = ActiveModel::new();
        configuration.set(params);
        Ok(configuration.insert(db).await?)
    }

    /// # Errors
    /// Tera's message, when a template doesn't compile.
    pub fn compile(
        &self,
        operation: Operation,
        items: &[items::Model],
        artifacts_dir: &str,
    ) -> std::result::Result<String, String> {
        Params::from(self).compile(operation, items, artifacts_dir)
    }
}

impl ActiveModel {
    fn set(&mut self, params: &Params) {
        self.name = Set(params.name.trim().to_string());
        self.system_message = Set(params.system_message.clone());
        self.triage_prompt = Set(params.triage_prompt.clone());
        self.review_prompt = Set(params.review_prompt.clone());
        self.develop_prompt = Set(params.develop_prompt.clone());
    }

    /// # Errors
    /// When the update fails.
    pub async fn revised(mut self, db: &DatabaseConnection, params: &Params) -> ModelResult<Model> {
        self.set(params);
        Ok(self.update(db).await?)
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
