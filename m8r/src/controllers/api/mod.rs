//! The JSON API, under `/api`. A session or a token lets a caller in, as it does to the pages,
//! and a token's scope says which resources its holder can use. The access guard checks both
//! (see `initializers::access`), so nothing here does.

use loco_rs::prelude::*;
use serde_json::Value;

use crate::models::Checked;

pub mod repos;
pub mod runs;
pub mod schedules;
pub mod watchlist_items;

/// What a change answers: the thing as it's saved, or a 400 with the reason, which is the
/// caller's to fix.
fn saved<T>(checked: Checked<T>, view: impl FnOnce(&T) -> Value) -> Result<Response> {
    match checked {
        Ok(it) => format::json(view(&it)),
        Err(problem) => bad_request(problem),
    }
}
