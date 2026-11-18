//! Which repos a schedule runs in: a row for each.

pub use super::_entities::schedule_repos::{ActiveModel, Column, Entity, Model};
use sea_orm::entity::prelude::*;

pub type ScheduleRepos = Entity;

impl ActiveModelBehavior for ActiveModel {}
