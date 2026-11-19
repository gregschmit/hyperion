use std::{
    fs::{self, File, TryLockError},
    io::Read,
    path::Path,
    time::Duration,
};

use jiff::Zoned;
use loco_rs::prelude::*;
use tokio::task::JoinSet;

use crate::{
    initializers::access,
    models::{errors, repos, runs, schedules, watchlist_items, Checked},
    prompts,
    settings::{self, Settings},
    tools,
};

/// How much m8r reads of what a run printed.
const OUTPUT_BYTES: u64 = 1024 * 1024;

/// How long the pages get to take a connection.
const PAGES_LIMIT: Duration = Duration::from_secs(3);

/// The scheduler runs this every minute, each time in a process of its own. m8r's schedules
/// are rows, not scheduler jobs, so this is where they're read.
pub struct Tick;
#[async_trait]
impl Task for Tick {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "tick".to_string(),
            detail: "Queue the schedules that are due, look at the repos, ask the forges about \
                     the watchlist, and start the runs of the schedule that has waited the \
                     longest. The scheduler runs this every minute."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let settings = Settings::from_context(ctx)?;
        fs::create_dir_all(&settings.data)?;
        // The pages say so when this file gets old: the scheduler isn't running.
        fs::write(settings.tick_file(), "")?;

        // The pages raise this error when the ticks stop.
        errors::Model::clear(&ctx.db, errors::SCHEDULER).await?;
        errors::Model::check_setup(&ctx.db).await?;
        errors::Model::forget(&ctx.db).await?;
        repos::Model::check(&ctx.db, &settings.shell).await?;

        queue_due(ctx).await?;

        // One tick at a time talks to the forges: a slow answer mustn't pile ticks up.
        if let Some(_checking) = lock(&settings.data.join("check.lock"))? {
            watchlist_items::Model::refresh(&ctx.db, &settings.shell).await?;
            check_api(ctx).await?;
        }

        // The tick that has runs in progress holds this lock for as long as they take.
        let Some(_running) = lock(&settings.lock_file())? else {
            return Ok(());
        };
        // So nothing is running now, whatever the rows say.
        runs::Model::fail_running(&ctx.db, "m8r stopped while the run was in progress").await?;

        // One schedule's runs start together, one in each of its repos. Another schedule's
        // wait for a later tick, so two schedules never work in the same clone at once.
        let mut running = JoinSet::new();
        for run in runs::Model::next_batch(&ctx.db).await? {
            running.spawn(execute(ctx.clone(), settings.clone(), run));
        }
        // Each run is on its own: one that goes wrong doesn't end the others.
        let mut ended = Ok(());
        while let Some(run) = running.join_next().await {
            if let Err(err) = run.map_err(Error::wrap).and_then(|run| run) {
                ended = Err(err);
            }
        }
        ended
    }
}

/// Take the lock file at `path`, or nothing if another process holds it. The lock goes when
/// the file is dropped, or when the process dies.
fn lock(path: &Path) -> Result<Option<File>> {
    let file = File::create(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(err)) => Err(err.into()),
    }
}

/// Queue the runs of each schedule whose time has come: one for each of its repos.
async fn queue_due(ctx: &AppContext) -> Result<()> {
    // The server's clock: a schedule's times are on it, unless the schedule names a time zone.
    let now = Zoned::now();
    for (schedule, repos) in schedules::Model::list(&ctx.db).await? {
        if schedule.is_due(&now) {
            runs::Model::queue(&ctx.db, &schedule, &repos).await?;
        }
    }
    Ok(())
}

/// See whether a run can reach m8r's API: a tick runs as the same user, on the same host. If
/// it can't, an agent can't change the watchlist, and nothing else would say so.
async fn check_api(ctx: &AppContext) -> Result<()> {
    let server = &ctx.config.server;
    let address = (
        server.binding.as_str(),
        u16::try_from(server.port).unwrap_or_default(),
    );
    let connection = tokio::net::TcpStream::connect(address);
    let problem = match tokio::time::timeout(PAGES_LIMIT, connection).await {
        Ok(Ok(_)) => None,
        Ok(Err(err)) => Some(err.to_string()),
        Err(_) => Some("no answer in time".to_string()),
    };
    let message = problem.map(|problem| {
        format!(
            "A run can't reach m8r's API at {}, so its agent can't change the watchlist: \
             {problem}. The pages are down, or a firewall keeps m8r's user from them.",
            settings::local_url(ctx)
        )
    });
    Ok(errors::Model::set(&ctx.db, errors::API, message.as_deref()).await?)
}

/// Run one queued run from start to end, and record how it went.
async fn execute(ctx: AppContext, settings: Settings, run: runs::Model) -> Result<()> {
    let ctx = &ctx;
    let (schedule, repo, prompt) = match prepare(ctx, &run).await? {
        Ok(ready) => ready,
        Err(why) => {
            run.into_active_model()
                .finished(&ctx.db, runs::Status::Failed, String::new(), why)
                .await?;
            return Ok(());
        }
    };
    let run = run
        .into_active_model()
        .started(&ctx.db, &schedule, &repo, prompt.clone())
        .await?;

    // What m8r's part of the prompt tells the agent to use (see `prompts`). The token ends
    // with the run.
    let limit = settings.run_limit();
    let env = [
        ("M8R_URL", settings::local_url(ctx)),
        (
            "M8R_TOKEN",
            access::run_token(ctx, run.id, limit.as_secs() + 60)?,
        ),
    ];
    // `task output` reads this file while the run is in progress.
    let printed = settings.output_file(run.id);
    let end = tools::run_command(
        &settings.shell,
        &run.command,
        &repo.dir(),
        &prompt,
        &printed,
        limit,
        &env,
    )
    .await;
    let output = read_start(&printed);
    let _ = fs::remove_file(&printed);

    let problem = match end {
        Ok(end) => end.problem(limit),
        Err(err) => Some(format!("m8r couldn't start the command: {err}")),
    };
    let status = if problem.is_some() {
        runs::Status::Failed
    } else {
        runs::Status::Ok
    };
    run.into_active_model()
        .finished(&ctx.db, status, output, problem.unwrap_or_default())
        .await?;
    Ok(())
}

/// The run's schedule and its repo as they are now, and its compiled prompt, which the command
/// gets on stdin. Or why the run can't start.
async fn prepare(
    ctx: &AppContext,
    run: &runs::Model,
) -> Result<Checked<(schedules::Model, repos::Model, String)>> {
    let Some(schedule_id) = run.schedule_id else {
        return Ok(Err(
            "The run's schedule was removed before the run started.".to_string(),
        ));
    };
    let Some(repo_id) = run.repo_id else {
        return Ok(Err(
            "The run's repo was removed before the run started.".to_string()
        ));
    };
    let (schedule, _) = schedules::Model::find(&ctx.db, schedule_id).await?;
    let repo = repos::Model::find(&ctx.db, repo_id).await?;
    if !repo.dir().is_dir() {
        return Ok(Err(format!("{} isn't a directory.", repo.path)));
    }
    let (items, max_items) = (schedule.items(), schedule.max_items());
    let context = prompts::Context::load(&ctx.db, Some(&repo), items, max_items).await?;
    Ok(context
        .compile(&schedule.prompt)
        .map(|prompt| (schedule, repo, prompt)))
}

/// The start of what the run printed: all of it, unless it's huge.
fn read_start(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = File::open(path) {
        let _ = file.take(OUTPUT_BYTES).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
