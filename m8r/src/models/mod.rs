pub mod _entities;
pub mod artifacts;
pub mod categories;
pub mod configurations;
pub mod runs;
pub mod schedules;
pub mod tokens;
pub mod tool_errors;
pub mod used_links;
pub mod watchlist_items;

/// What a change gives back: the thing as it's saved, or why it can't be saved. That's for
/// whoever asked to fix, so the pages, the API and the MCP tools say it back.
pub type Checked<T> = std::result::Result<T, String>;
