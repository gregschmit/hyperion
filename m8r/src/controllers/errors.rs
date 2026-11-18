#![allow(clippy::missing_errors_doc)]
use loco_rs::prelude::*;
use serde_json::json;

use crate::{models::errors, views};

/// How many cleared errors the page lists.
const CLEARED: u64 = 200;

/// What's wrong now, and what was: the open errors, and then the ones that were cleared.
#[debug_handler]
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    // Look first, so that the list has what this look raises and clears.
    views::look(&ctx).await?;
    let listed: Vec<_> = errors::Model::list(&ctx.db, CLEARED)
        .await?
        .iter()
        .map(views::error)
        .collect();
    let data = json!({ "errors": listed });
    views::page(&v, &ctx, "errors/index.html", "errors", data).await
}

pub fn routes() -> Routes {
    Routes::new().prefix("/errors").add("/", get(index))
}
