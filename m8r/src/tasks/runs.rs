use loco_rs::prelude::*;

use crate::{models::runs, views};

pub struct Runs;
#[async_trait]
impl Task for Runs {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "runs".to_string(),
            detail: "List the runs that aren't over, oldest first: each one's id, status, name \
                     and repo, and when it started. `task output id:12` prints what one prints."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let runs = runs::Model::unfinished(&ctx.db).await?;
        if runs.is_empty() {
            println!("No run is in progress, and none waits.");
        }
        for run in runs {
            // Blank for a run that waits.
            let started_at = run.started_at.as_ref().map(views::time).unwrap_or_default();
            println!(
                "{}\t{}\t{}\t{}\t{started_at}",
                run.id, run.status, run.name, run.repo
            );
        }
        Ok(())
    }
}
