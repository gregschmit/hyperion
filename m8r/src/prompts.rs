//! What a run's command gets on stdin: its schedule's prompt.
//!
//! A prompt is a Tera template, and so is the prompt of the active configuration. This is
//! what they can use, and how m8r compiles them.

use loco_rs::prelude::*;
use serde_json::json;

use crate::models::{categories, configurations, watchlist_items};

/// m8r's own part of a prompt: where the agent is, how it reports, and how it changes the
/// watchlist. A run's environment has the API's address and the token (see `tasks::tick`), so
/// the prompt holds neither.
const M8R_PROMPT: &str = "\
You are running in m8r, a maintainer automation tool. Nobody is here to answer questions, so
decide for yourself.

When you finish, report what you made as artifacts. For each one, make a directory in
{artifacts_dir}, named for what it is. Put the artifact's files in it. If the artifact is
already online (a comment you posted, a PR/MR), put in one text file that holds only its link.

## How to use m8r's watchlist

m8r keeps the developer's watchlist: the issues and PRs/MRs that they follow. Each item has a
number, a category, a priority (medium: the developer cares about it; high: it's urgent), a
note, and for a PR/MR, whether someone asked the developer to review it (\"review requested\").

You can change the watchlist while you work, through m8r's API. Its address is in $M8R_URL, and
$M8R_TOKEN lets you in until this run ends. Both are in your environment.

List the items:

    curl -sS \"$M8R_URL/api/watchlist_items\" -H \"Authorization: Bearer $M8R_TOKEN\"

Add an item, or change the one that has this URL:

    curl -sS \"$M8R_URL/api/watchlist_items\" -H \"Authorization: Bearer $M8R_TOKEN\" \\
      --json '{\"url\": \"URL\", \"category\": \"CATEGORY\", \"priority\": \"medium\", \"note\": \"Why the developer follows it\"}'

Change an item, by its number:

    curl -sS -X PATCH \"$M8R_URL/api/watchlist_items/NUMBER\" -H \"Authorization: Bearer $M8R_TOKEN\" \\
      --json '{\"review_requested\": false}'

Remove an item, by its number:

    curl -sS -X DELETE \"$M8R_URL/api/watchlist_items/NUMBER\" -H \"Authorization: Bearer $M8R_TOKEN\"

The categories are: {categories}.

- Remove an item when its issue or PR/MR is closed or merged, or when nothing is left to follow.
- Add an item when you make something that the developer must follow, such as a PR/MR that
  waits for a review.
- When you have reviewed a PR/MR whose review is requested, set its `review_requested` to false.
- Leave the other items' priorities and notes as they are: they are the developer's.
";

/// What a new schedule's prompt starts as: m8r's part, yours, and the watchlist. After them,
/// you say what to do.
pub const STARTER: &str = "\
{{ m8r_prompt }}

{{ config_prompt }}

The watchlist now:
{{ watchlist }}

";

/// The artifact directory that a prompt is compiled with when no run is at hand.
pub const SAMPLE_ARTIFACTS_DIR: &str = "/var/lib/m8r/artifacts/1";

/// What a prompt can use, for the pages to say. A configuration's prompt can use all but
/// itself.
pub const VARIABLES: [(&str, &str); 6] = [
    (
        "{{ m8r_prompt }}",
        "m8r's own part: where the agent is, how it reports what it made, and how it adds, \
         changes and removes watchlist items while it runs.",
    ),
    (
        "{{ config_prompt }}",
        "The active configuration's prompt: your part of every prompt.",
    ),
    (
        "{{ watchlist }}",
        "The watchlist items in the schedule's category, one to a line. A schedule with no \
         category gets none.",
    ),
    (
        "{{ category }}",
        "The schedule's category. Blank with none.",
    ),
    (
        "{{ artifacts_dir }}",
        "The directory where the run reports its artifacts.",
    ),
    (
        "{% for item in watchlist_items %}…{% endfor %}",
        "The same items, to write the lines yourself. Each has id, url, category, priority, \
         note and review_requested.",
    ),
];

/// What a prompt is compiled with.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The active configuration's prompt, which is a template itself. Blank with no active
    /// configuration.
    pub config_prompt: String,
    /// The items that the run gets: the ones in its schedule's category. None, with no
    /// category.
    pub watchlist: Vec<watchlist_items::Listed>,
    /// The schedule's category. Blank with none.
    pub category: String,
    /// Every category, which m8r's part names.
    pub categories: Vec<String>,
    /// Where the run reports its artifacts.
    pub artifacts_dir: String,
}

impl Context {
    /// What a prompt gets now, for a schedule with this category (or none).
    ///
    /// # Errors
    /// When a query fails.
    pub async fn load(
        db: &DatabaseConnection,
        category: Option<&categories::Model>,
        artifacts_dir: &str,
    ) -> ModelResult<Self> {
        let active = configurations::Model::active(db).await?;
        Ok(Self {
            config_prompt: active.map(|it| it.prompt).unwrap_or_default(),
            watchlist: match category {
                Some(category) => watchlist_items::Model::list(db, Some(category.id)).await?,
                None => Vec::new(),
            },
            category: category.map(|it| it.name.clone()).unwrap_or_default(),
            categories: categories::Model::list(db)
                .await?
                .into_iter()
                .map(|it| it.name)
                .collect(),
            artifacts_dir: artifacts_dir.to_string(),
        })
    }

    /// Compile `prompt`: the configuration's prompt first, then the prompt, which says where
    /// that and m8r's part go.
    ///
    /// # Errors
    /// Tera's message, when a template doesn't compile.
    pub fn compile(&self, prompt: &str) -> std::result::Result<String, String> {
        // Not HTML: nothing in a prompt is escaped.
        let autoescape = false;
        let mut context = tera::Context::new();
        let items: Vec<_> = self
            .watchlist
            .iter()
            .map(|(item, category)| {
                json!({
                    "id": item.id,
                    "url": item.url,
                    "category": category.name,
                    "priority": item.priority,
                    "note": item.note,
                    "review_requested": item.review_requested,
                })
            })
            .collect();
        context.insert("watchlist_items", &items);
        context.insert("watchlist", &self.lines());
        context.insert("category", &self.category);
        context.insert("artifacts_dir", &self.artifacts_dir);
        context.insert("m8r_prompt", self.m8r_prompt().trim_end());
        let config_prompt = tera::Tera::one_off(&self.config_prompt, &context, autoescape)
            .map_err(|err| format!("The configuration's prompt doesn't compile.\n{err}"))?;
        context.insert("config_prompt", config_prompt.trim_end());
        tera::Tera::one_off(prompt, &context, autoescape)
            .map_err(|err| format!("The prompt doesn't compile.\n{err}"))
    }

    /// m8r's part, with this run's artifact directory and the categories as they are now.
    #[must_use]
    pub fn m8r_prompt(&self) -> String {
        M8R_PROMPT
            .replace("{artifacts_dir}", &self.artifacts_dir)
            .replace("{categories}", &self.categories.join(", "))
    }

    /// The watchlist as a prompt shows it: one item to a line.
    fn lines(&self) -> String {
        if self.watchlist.is_empty() {
            return "(nothing)".to_string();
        }
        let lines: Vec<String> = self
            .watchlist
            .iter()
            .map(|(item, category)| {
                let review = if item.review_requested {
                    ", review requested"
                } else {
                    ""
                };
                let line = format!(
                    "- item {} ({}, {}{review}): {} {}",
                    item.id, category.name, item.priority, item.url, item.note
                );
                line.trim_end().to_string()
            })
            .collect();
        lines.join("\n")
    }
}
