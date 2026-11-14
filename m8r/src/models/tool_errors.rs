pub use super::_entities::tool_errors::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;

pub type ToolErrors = Entity;

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

/// What's wrong right now with the tools that m8r runs: one row for each tool, and one for
/// each host that a tool can't work with. A row goes as soon as the tool works again.
impl Model {
    /// # Errors
    /// When the query fails.
    pub async fn list(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_asc(Column::Tool)
            .order_by_asc(Column::Host)
            .all(db)
            .await?)
    }

    /// Record what's wrong with `tool`, or with its dealings with `host`. The row keeps the
    /// time it first went wrong.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn set(
        db: &DatabaseConnection,
        tool: &str,
        host: &str,
        message: &str,
    ) -> ModelResult<()> {
        let txn = db.begin().await?;
        let known = Entity::find()
            .filter(Column::Tool.eq(tool))
            .filter(Column::Host.eq(host))
            .one(&txn)
            .await?;
        match known {
            Some(error) if error.message == message => {}
            Some(error) => {
                let mut error = error.into_active_model();
                error.message = Set(message.to_string());
                error.update(&txn).await?;
            }
            None => {
                let mut error = ActiveModel::new();
                error.tool = Set(tool.to_string());
                error.host = Set(host.to_string());
                error.message = Set(message.to_string());
                error.insert(&txn).await?;
            }
        }
        txn.commit().await?;
        Ok(())
    }

    /// The tool works again.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn clear(db: &DatabaseConnection, tool: &str, host: &str) -> ModelResult<()> {
        Entity::delete_many()
            .filter(Column::Tool.eq(tool))
            .filter(Column::Host.eq(host))
            .exec(db)
            .await?;
        Ok(())
    }
}
