pub use super::_entities::used_links::{ActiveModel, Column, Entity, Model};
use chrono::{DateTime, Utc};
use loco_rs::prelude::*;

pub type UsedLinks = Entity;

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
    /// Use a single-use access link. False when it was used before.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn spend(
        db: &DatabaseConnection,
        token_id: &str,
        expires_at: DateTime<Utc>,
    ) -> ModelResult<bool> {
        let txn = db.begin().await?;
        // A link that would have expired anyway needs no row to refuse it.
        Entity::delete_many()
            .filter(Column::ExpiresAt.lt(Utc::now()))
            .exec(&txn)
            .await?;
        let used = Entity::find()
            .filter(Column::TokenId.eq(token_id))
            .one(&txn)
            .await?;
        if used.is_some() {
            return Ok(false);
        }
        let mut link = ActiveModel::new();
        link.token_id = Set(token_id.to_string());
        link.expires_at = Set(expires_at.into());
        link.insert(&txn).await?;
        txn.commit().await?;
        Ok(true)
    }
}
