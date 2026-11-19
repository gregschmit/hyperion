pub use super::_entities::tokens::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;
use sha2::{Digest, Sha256};

use super::Checked;

pub type Tokens = Entity;

/// What a token can cover. Each is a part of the API, `/api/<resource>`, and a set of the MCP
/// endpoint's tools.
pub const RESOURCES: [&str; 4] = ["watchlist_items", "repos", "schedules", "runs"];

/// The scope that covers every resource.
pub const ALL: &str = "all";

/// How a token starts, so that one that leaks is easy to recognize.
pub const PREFIX: &str = "m8r_";

/// Whether `scope` covers `resource`. A scope is `all`, or resources with commas between
/// them.
#[must_use]
pub fn covers(scope: &str, resource: &str) -> bool {
    scope == ALL || scope.split(',').any(|covered| covered == resource)
}

/// A scope as it's kept, or why it isn't one.
fn scope(text: &str) -> Checked<String> {
    let resources: Vec<&str> = text
        .split(',')
        .map(str::trim)
        .filter(|resource| !resource.is_empty())
        .collect();
    let known = |resource: &&str| *resource == ALL || RESOURCES.contains(resource);
    if resources.is_empty() || !resources.iter().all(known) {
        return Err(format!(
            "A scope is {ALL}, or one or more of these with commas between them: {}.",
            RESOURCES.join(", ")
        ));
    }
    if resources.contains(&ALL) {
        return Ok(ALL.to_string());
    }
    Ok(resources.join(","))
}

/// What m8r keeps of a token: its SHA-256. The token is long and random, so the digest needs
/// no salt, and it finds the row.
fn digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
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
        Ok(Entity::find().order_by_asc(Column::Name).all(db).await?)
    }

    /// Make a token called `name` that covers `scope`, and give the token itself. This is the
    /// only time that anyone sees it.
    ///
    /// # Errors
    /// When a query fails.
    pub async fn create(
        db: &DatabaseConnection,
        name: &str,
        wanted: &str,
    ) -> ModelResult<Checked<String>> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(Err(
                "Give the token a name, for what will use it.".to_string()
            ));
        }
        let scope = match scope(wanted) {
            Ok(scope) => scope,
            Err(problem) => return Ok(Err(problem)),
        };
        let taken = Entity::find().filter(Column::Name.eq(name)).one(db).await?;
        if taken.is_some() {
            return Ok(Err(format!(
                "There's already a token called {name}. Revoke it first."
            )));
        }
        let token = format!("{PREFIX}{}", loco_rs::hash::random_string(40));
        let mut row = ActiveModel::new();
        row.name = Set(name.to_string());
        row.digest = Set(digest(&token));
        row.scope = Set(scope);
        row.insert(db).await?;
        Ok(Ok(token))
    }

    /// The row of `token`, if it's one of m8r's.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn of(db: &DatabaseConnection, token: &str) -> ModelResult<Option<Self>> {
        Ok(Entity::find()
            .filter(Column::Digest.eq(digest(token)))
            .one(db)
            .await?)
    }

    /// End the token called `name`. False when there's none.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn revoke(db: &DatabaseConnection, name: &str) -> ModelResult<bool> {
        let gone = Entity::delete_many()
            .filter(Column::Name.eq(name.trim()))
            .exec(db)
            .await?;
        Ok(gone.rows_affected > 0)
    }
}
