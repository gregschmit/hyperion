use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub use super::_entities::artifacts::{ActiveModel, Column, Entity, Model};
use loco_rs::prelude::*;

pub type Artifacts = Entity;

/// The directory that gathers the files an agent left loose in its artifact directory.
const LOOSE: &str = "report";

/// A link file is one short line.
const LINK_BYTES: u64 = 4096;

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
    /// The latest artifacts, newest first.
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
    /// When the query fails.
    pub async fn of_run(db: &DatabaseConnection, run_id: i64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::RunId.eq(run_id))
            .order_by_asc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// `ModelError::EntityNotFound` when there's no such artifact.
    pub async fn find(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| ModelError::EntityNotFound)
    }

    /// m8r hosts this artifact's files. The other kind is a link.
    #[must_use]
    pub fn hosted(&self) -> bool {
        self.url.is_empty()
    }

    /// Where a hosted artifact's files are, given its run's artifact directory.
    #[must_use]
    pub fn dir(&self, run_dir: &Path) -> PathBuf {
        run_dir.join(&self.name)
    }

    /// Record what a run left in its artifact directory `dir`: one artifact for each directory
    /// there. A directory that holds only a link is a link, and any other one is a set of
    /// files, which stay where they are.
    ///
    /// # Errors
    /// When the directory can't be read, or a query fails.
    pub async fn collect(
        db: &DatabaseConnection,
        run_id: i64,
        dir: &Path,
    ) -> ModelResult<Vec<Self>> {
        let names = reported(dir).map_err(|err| ModelError::Any(err.into()))?;
        let known: Vec<String> = Self::of_run(db, run_id)
            .await?
            .into_iter()
            .map(|artifact| artifact.name)
            .collect();
        for name in names {
            if known.contains(&name) {
                continue;
            }
            let mut artifact = ActiveModel::new();
            artifact.run_id = Set(run_id);
            artifact.url = Set(link(&dir.join(&name)).unwrap_or_default());
            artifact.name = Set(name);
            artifact.insert(db).await?;
        }
        Self::of_run(db, run_id).await
    }
}

/// The names of the artifact directories in `dir`, which may not exist.
fn reported(dir: &Path) -> io::Result<Vec<String>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    gather_loose(dir)?;
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // Not through a symlink: an artifact is what's in the directory.
        let is_dir = entry.file_type()?.is_dir();
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if is_dir && !name.starts_with('.') && fs::read_dir(entry.path())?.next().is_some() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// An agent that didn't make a directory for its artifact left its files loose. Put them in
/// one, so they're reported as one artifact.
fn gather_loose(dir: &Path) -> io::Result<()> {
    let mut loose = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && !entry.file_name().to_string_lossy().starts_with('.') {
            loose.push(entry.file_name());
        }
    }
    if loose.is_empty() {
        return Ok(());
    }
    let mut target = dir.join(LOOSE);
    let mut count = 1;
    while target.exists() {
        count += 1;
        target = dir.join(format!("{LOOSE}-{count}"));
    }
    fs::create_dir(&target)?;
    for name in loose {
        fs::rename(dir.join(&name), target.join(&name))?;
    }
    Ok(())
}

/// The link that an artifact's directory holds, if a link is all it holds: one small text file
/// with nothing in it but an `http://` or `https://` URL.
fn link(dir: &Path) -> Option<String> {
    let mut entries = fs::read_dir(dir).ok()?;
    let only = entries.next()?.ok()?;
    if entries.next().is_some() {
        return None;
    }
    let meta = only.metadata().ok()?;
    if !only.file_type().ok()?.is_file() || meta.len() > LINK_BYTES {
        return None;
    }
    let text = fs::read_to_string(only.path()).ok()?;
    let url = text.trim();
    let web = url.starts_with("https://") || url.starts_with("http://");
    (web && !url.contains(char::is_whitespace)).then(|| url.to_string())
}
