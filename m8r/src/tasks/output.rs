use std::{
    fs::File,
    io::{self, Write},
    time::Duration,
};

use loco_rs::prelude::*;

use crate::{
    models::runs::{self, Status},
    settings::Settings,
};

/// How long `follow` waits before it looks for more.
const PAUSE: Duration = Duration::from_millis(500);

pub struct Output;
#[async_trait]
impl Task for Output {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "output".to_string(),
            detail: "Print what a run printed: id:12. While the run is in progress, it prints \
                     what comes, until the run ends. `task runs` lists the runs that aren't over."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let id = vars
            .cli_arg("id")
            .ok()
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| Error::string("say which run, such as id:12"))?;
        let run = follow(ctx, id, &mut io::stdout()).await?;
        // On stderr: stdout has what the run printed, and nothing else.
        if run.error.is_empty() {
            eprintln!("Run {id}: {}", run.status);
        } else {
            eprintln!("Run {id}: {}: {}", run.status, run.error);
        }
        Ok(())
    }
}

/// Write what run `id` printed to `out`, and give the run back when it's over. While a run is
/// in progress, its command prints to a file (see `tasks::tick`), and this writes what comes
/// there. A run that's over has what it printed in its row.
///
/// # Errors
/// When there's no such run, or when a query or `out` fails.
pub async fn follow(ctx: &AppContext, id: i64, out: &mut impl Write) -> Result<runs::Model> {
    let path = Settings::from_context(ctx)?.output_file(id);
    let mut file: Option<File> = None;
    loop {
        let run = match runs::Model::find(&ctx.db, id).await {
            Err(ModelError::EntityNotFound) => {
                return Err(Error::string(&format!("there's no run {id}")))
            }
            found => found?,
        };
        let over = [Status::Ok.as_str(), Status::Failed.as_str()].contains(&run.status.as_str());
        if file.is_none() && !over {
            // Not there for a run that waits, or one that starts just now.
            file = File::open(&path).ok();
        }
        match &mut file {
            // What's new in it. The tick removes the file when the run ends, and an open file
            // still has all of it.
            Some(file) => {
                io::copy(file, out)?;
            }
            None if over => out.write_all(run.output.as_bytes())?,
            None => {}
        }
        out.flush()?;
        if over {
            return Ok(run);
        }
        tokio::time::sleep(PAUSE).await;
    }
}
