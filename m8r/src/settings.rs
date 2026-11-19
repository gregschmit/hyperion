use std::{path::PathBuf, time::Duration};

use loco_rs::prelude::*;
use serde::Deserialize;

/// m8r's own settings: the `settings:` block of `config/<env>.yaml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    /// Where the app is reached, for the access links.
    pub url: String,
    /// The shell that starts the tools, as a login shell.
    pub shell: String,
    /// The agent's command, which a new schedule starts with. A run pipes its prompt to its
    /// schedule's own command.
    pub agent: String,
    /// Minutes before a run is killed.
    pub run_limit: u64,
    /// Where the artifacts and the run lock go.
    pub data: PathBuf,
}

impl Settings {
    /// # Errors
    /// When the `settings:` block is missing a key.
    pub fn from_context(ctx: &AppContext) -> Result<Self> {
        ctx.config.settings::<Self>()
    }

    #[must_use]
    pub fn run_limit(&self) -> Duration {
        Duration::from_secs(self.run_limit * 60)
    }

    /// The directory where run `id` reports its artifacts.
    #[must_use]
    pub fn artifacts_dir(&self, run_id: i64) -> PathBuf {
        self.data.join("artifacts").join(run_id.to_string())
    }

    /// The file that gets what run `id` prints, for as long as the run takes.
    #[must_use]
    pub fn output_file(&self, run_id: i64) -> PathBuf {
        self.artifacts_dir(run_id).with_extension("out")
    }

    /// The file that the tick that has a run in progress keeps locked.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.data.join("run.lock")
    }

    /// The file that every tick touches, so the pages can tell that the scheduler runs.
    #[must_use]
    pub fn tick_file(&self) -> PathBuf {
        self.data.join("tick")
    }
}

/// Where a run reaches m8r's API: the address that the pages listen on, on this host.
#[must_use]
pub fn local_url(ctx: &AppContext) -> String {
    let server = &ctx.config.server;
    format!("http://{}:{}", server.binding, server.port)
}
