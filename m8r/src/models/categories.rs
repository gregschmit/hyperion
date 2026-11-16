pub use super::_entities::categories::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;
use serde::Deserialize;

use super::{schedules, watchlist_items, Checked};

pub type Categories = Entity;

/// How long a category's name can be.
pub const NAME_CHARS: usize = 255;

/// What a category's forms and the API send.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Params {
    pub name: String,
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
    /// Every category, in the order they were made.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find().order_by_asc(Column::Id).all(db).await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such category.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// The category with this name, or why there's none: the message names the ones there are,
    /// so whoever asked can choose.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn named<C: ConnectionTrait>(db: &C, name: &str) -> ModelResult<Checked<Self>> {
        let name = name.trim();
        let found = Entity::find().filter(Column::Name.eq(name)).one(db).await?;
        if let Some(category) = found {
            return Ok(Ok(category));
        }
        let names: Vec<String> = Entity::find()
            .order_by_asc(Column::Id)
            .all(db)
            .await?
            .into_iter()
            .map(|category| category.name)
            .collect();
        let wanted = if name.is_empty() {
            "Choose a category.".to_string()
        } else {
            format!("There's no category \"{name}\".")
        };
        Ok(Err(format!(
            "{wanted} The categories are: {}.",
            names.join(", ")
        )))
    }

    /// # Errors
    /// When a query fails.
    pub async fn create(db: &DatabaseConnection, params: &Params) -> ModelResult<Checked<Self>> {
        let name = match Self::free_name(db, &params.name, None).await? {
            Ok(name) => name,
            Err(problem) => return Ok(Err(problem)),
        };
        let mut category = ActiveModel::new();
        category.name = Set(name);
        Ok(Ok(category.insert(db).await?))
    }

    /// # Errors
    /// When a query fails.
    pub async fn renamed(
        self,
        db: &DatabaseConnection,
        params: &Params,
    ) -> ModelResult<Checked<Self>> {
        let name = match Self::free_name(db, &params.name, Some(self.id)).await? {
            Ok(name) => name,
            Err(problem) => return Ok(Err(problem)),
        };
        let mut category = self.into_active_model();
        category.name = Set(name);
        Ok(Ok(category.update(db).await?))
    }

    /// Remove the category, unless a watchlist item or a schedule uses it.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn removed(self, db: &DatabaseConnection) -> ModelResult<Checked<()>> {
        let items = watchlist_items::Entity::find()
            .filter(watchlist_items::Column::CategoryId.eq(self.id))
            .count(db)
            .await?;
        let schedules = schedules::Entity::find()
            .filter(schedules::Column::CategoryId.eq(self.id))
            .count(db)
            .await?;
        if items + schedules > 0 {
            return Ok(Err(format!(
                "\"{}\" is in use: by {items} watchlist items and {schedules} schedules. Give \
                 them another category first.",
                self.name
            )));
        }
        self.delete(db).await?;
        Ok(Ok(()))
    }

    /// The name as it's kept, or why it can't be: blank, too long, or another category's.
    async fn free_name(
        db: &DatabaseConnection,
        name: &str,
        own_id: Option<i64>,
    ) -> ModelResult<Checked<String>> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(Err("Give the category a name.".to_string()));
        }
        if name.chars().count() > NAME_CHARS {
            return Ok(Err(format!(
                "A category's name can have {NAME_CHARS} characters."
            )));
        }
        let taken = Entity::find()
            .filter(Column::Name.eq(name))
            .one(db)
            .await?
            .is_some_and(|other| Some(other.id) != own_id);
        if taken {
            return Ok(Err(format!("There's already a category \"{name}\".")));
        }
        Ok(Ok(name.to_string()))
    }
}
