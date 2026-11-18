//! What a run's command gets on stdin: its schedule's prompt.
//!
//! A prompt is a Tera template. This is what it can use, and how m8r compiles it. The active
//! configuration's prompt is one of those things: plain text, which no one compiles.

use loco_rs::prelude::*;
use serde_json::{json, Value};

use crate::models::{configurations, repos, schedules::Items, watchlist_items};

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

m8r keeps the developer's watchlist: the issues and PRs/MRs that they follow, each in one of
the repos that m8r works in. You work in one repo, and its items are yours. Each item has a
number, a priority (medium: the developer cares about it; high: it's urgent), a note, and
whether someone asked the developer for it (\"requested\"): for a PR/MR, a review.

You can change the watchlist while you work, through m8r's API. Its address is in $M8R_URL, and
$M8R_TOKEN lets you in until this run ends. Both are in your environment.

List your repo's items:

    curl -sS \"$M8R_URL/api/watchlist_items{of_repo}\" -H \"Authorization: Bearer $M8R_TOKEN\"

Add an item, or change the one that has this URL:

    curl -sS \"$M8R_URL/api/watchlist_items\" -H \"Authorization: Bearer $M8R_TOKEN\" \\
      --json '{\"url\": \"URL\", \"priority\": \"medium\", \"note\": \"Why the developer follows it\"}'

Change an item, by its number:

    curl -sS -X PATCH \"$M8R_URL/api/watchlist_items/NUMBER\" -H \"Authorization: Bearer $M8R_TOKEN\" \\
      --json '{\"requested\": false}'

Remove an item, by its number:

    curl -sS -X DELETE \"$M8R_URL/api/watchlist_items/NUMBER\" -H \"Authorization: Bearer $M8R_TOKEN\"

- Remove an item when its issue or PR/MR is closed or merged, or when nothing is left to follow.
- Add an item when you make something that the developer must follow, such as a PR/MR that
  waits for a review. Its URL says which repo it's in, and m8r takes it only for a repo that
  it works in.
- When you have done what a requested item asks for, such as the review of a PR/MR, set its
  `requested` to false.
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

/// What a prompt can use, for the pages to say.
pub const VARIABLES: [(&str, &str); 10] = [
    (
        "{{ m8r_prompt }}",
        "m8r's own part: where the agent is, how it reports what it made, and how it adds, \
         changes and removes watchlist items while it runs.",
    ),
    (
        "{{ config_prompt }}",
        "The active configuration's prompt: your part of every prompt, as you wrote it.",
    ),
    (
        "{{ watchlist }}",
        "The watchlist items that the run gets, one to a line: its repo's, of the kind that \
         the schedule takes. The requested ones come first, then the urgent ones.",
    ),
    (
        "{{ requested_watchlist }}",
        "Only the requested ones of those items, one to a line.",
    ),
    ("{{ repo }}", "The run's repo: the path of its clone."),
    (
        "{{ repo_url }}",
        "The repo's project on its forge. Blank if m8r doesn't know it.",
    ),
    (
        "{{ category }}",
        "Which items the schedule takes: all, issues or pull/merge requests. Blank with none.",
    ),
    (
        "{{ artifacts_dir }}",
        "The directory where the run reports its artifacts.",
    ),
    (
        "{% for item in watchlist_items %}…{% endfor %}",
        "The same items, to write the lines yourself. Each has id, url, category, priority, \
         note and requested.",
    ),
    (
        "{% for item in requested_watchlist_items %}…{% endfor %}",
        "Only the requested ones, the same way.",
    ),
];

/// What a prompt is compiled with.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The active configuration's prompt: plain text, which goes in as it is. Blank with no
    /// active configuration.
    pub config_prompt: String,
    /// The items that the run gets: its repo's, of the kind that its schedule takes, in the
    /// order that a run works them in. None with no repo, or for a schedule that takes none.
    pub watchlist: Vec<watchlist_items::Model>,
    /// Which items the schedule takes. Blank with none.
    pub category: String,
    /// The run's repo. None when no run is at hand, and no repo is chosen yet.
    pub repo: Option<repos::Model>,
    /// Where the run reports its artifacts.
    pub artifacts_dir: String,
}

impl Context {
    /// What a prompt gets now, for a run in `repo` of a schedule that takes `items`, and
    /// `most` of them at most (0: all of them).
    ///
    /// # Errors
    /// When a query fails.
    pub async fn load(
        db: &DatabaseConnection,
        repo: Option<&repos::Model>,
        items: Items,
        most: u32,
        artifacts_dir: &str,
    ) -> ModelResult<Self> {
        let active = configurations::Model::active(db).await?;
        let category = match items {
            Items::None => None,
            Items::All => Some(None),
            Items::Of(category) => Some(Some(category)),
        };
        let watchlist = match (repo, category) {
            (Some(repo), Some(category)) => {
                watchlist_items::Model::of_repo(db, repo.id, category, most.into()).await?
            }
            _ => Vec::new(),
        };
        Ok(Self {
            config_prompt: active.map(|it| it.prompt).unwrap_or_default(),
            watchlist,
            category: items.as_str().to_string(),
            repo: repo.cloned(),
            artifacts_dir: artifacts_dir.to_string(),
        })
    }

    /// Compile `prompt`, which says where m8r's part and the configuration's go.
    ///
    /// # Errors
    /// Tera's message, when the prompt doesn't compile.
    pub fn compile(&self, prompt: &str) -> std::result::Result<String, String> {
        // Not HTML: nothing in a prompt is escaped.
        let autoescape = false;
        let requested: Vec<&watchlist_items::Model> = self
            .watchlist
            .iter()
            .filter(|item| item.requested)
            .collect();
        let (repo, repo_url) = self
            .repo
            .as_ref()
            .map(|repo| (repo.path.as_str(), repo.url.as_str()))
            .unwrap_or_default();
        let mut context = tera::Context::new();
        context.insert("watchlist_items", &items(self.watchlist.iter()));
        context.insert("watchlist", &lines(self.watchlist.iter()));
        context.insert(
            "requested_watchlist_items",
            &items(requested.iter().copied()),
        );
        context.insert("requested_watchlist", &lines(requested.iter().copied()));
        context.insert("category", &self.category);
        context.insert("repo", repo);
        context.insert("repo_url", repo_url);
        context.insert("artifacts_dir", &self.artifacts_dir);
        context.insert("m8r_prompt", self.m8r_prompt().trim_end());
        // Plain text: a value goes in as it is, whatever braces it has.
        context.insert("config_prompt", self.config_prompt.trim_end());
        tera::Tera::one_off(prompt, &context, autoescape)
            .map_err(|err| format!("The prompt doesn't compile.\n{err}"))
    }

    /// m8r's part, with this run's artifact directory, and its repo for the list of items.
    #[must_use]
    pub fn m8r_prompt(&self) -> String {
        let of_repo = self
            .repo
            .as_ref()
            .map(|repo| format!("?repo_id={}", repo.id))
            .unwrap_or_default();
        M8R_PROMPT
            .replace("{artifacts_dir}", &self.artifacts_dir)
            .replace("{of_repo}", &of_repo)
    }
}

/// Items as a template loops over them.
fn items<'a>(items: impl Iterator<Item = &'a watchlist_items::Model>) -> Vec<Value> {
    items
        .map(|item| {
            json!({
                "id": item.id,
                "url": item.url,
                "category": item.category,
                "priority": item.priority,
                "note": item.note,
                "requested": item.requested,
            })
        })
        .collect()
}

/// Items as a prompt shows them: one to a line. The URL says which repo an item is in, and
/// whether it's an issue or a PR/MR.
fn lines<'a>(items: impl Iterator<Item = &'a watchlist_items::Model>) -> String {
    let lines: Vec<String> = items
        .map(|item| {
            let requested = if item.requested { ", requested" } else { "" };
            let line = format!(
                "- item {} ({}{requested}): {} {}",
                item.id, item.priority, item.url, item.note
            );
            line.trim_end().to_string()
        })
        .collect();
    if lines.is_empty() {
        return "(nothing)".to_string();
    }
    lines.join("\n")
}
