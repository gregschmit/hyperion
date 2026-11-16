use std::{
    collections::BTreeSet,
    fs::{self, File, TryLockError},
    io::Read,
    path::Path,
    time::Duration,
};

use jiff::Zoned;
use loco_rs::prelude::*;

use crate::{
    initializers::access,
    models::{artifacts, configurations, runs, schedules, tool_errors, watchlist_items, Checked},
    prompts,
    settings::{self, Settings},
    tools,
};

/// How much m8r reads of what a run printed.
const OUTPUT_BYTES: u64 = 1024 * 1024;

/// What the pages are called in the list of what doesn't work.
const PAGES: &str = "m8r-web";

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
            detail: "Queue the schedules that are due, check the tools and the watchlist, and \
                     start the oldest queued run. The scheduler runs this every minute."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let settings = Settings::from_context(ctx)?;
        fs::create_dir_all(&settings.data)?;
        // The pages say so when this file gets old: the scheduler isn't running.
        fs::write(settings.tick_file(), "")?;

        queue_due(ctx).await?;

        // One tick at a time talks to the forges: a slow answer mustn't pile ticks up.
        if let Some(_checking) = lock(&settings.data.join("check.lock"))? {
            check_tools(ctx, &settings).await?;
        }

        // The tick that has a run in progress holds this lock for as long as the run takes.
        let Some(_running) = lock(&settings.lock_file())? else {
            return Ok(());
        };
        // So nothing is running now, whatever the rows say.
        runs::Model::fail_running(&ctx.db, "m8r stopped while the run was in progress").await?;
        if let Some(run) = runs::Model::next(&ctx.db).await? {
            execute(ctx, &settings, run).await?;
        }
        Ok(())
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

/// Queue a run for each schedule whose time has come. With no active configuration, nothing
/// runs.
async fn queue_due(ctx: &AppContext) -> Result<()> {
    if configurations::Model::active(&ctx.db).await?.is_none() {
        return Ok(());
    }
    // The server's clock: a schedule's times are on it, unless the schedule names a time zone.
    let now = Zoned::now();
    for (schedule, _) in schedules::Model::list(&ctx.db).await? {
        if schedule.is_due(&now) {
            runs::Model::queue(&ctx.db, &schedule).await?;
        }
    }
    Ok(())
}

/// See whether the tools work: the programs that the schedules' commands start, `gh` and
/// `glab`, which ask the forges about the watchlist, and m8r's own API, which the agents use.
/// A tool that's missing or logged out is recorded, not fatal, and the record goes when it
/// works.
async fn check_tools(ctx: &AppContext, settings: &Settings) -> Result<()> {
    // The agent's own command too: before there's a schedule, setup wants to know.
    let mut programs: BTreeSet<String> = tools::program(&settings.agent)
        .map(str::to_string)
        .into_iter()
        .collect();
    for (schedule, _) in schedules::Model::list(&ctx.db).await? {
        programs.extend(tools::program(&schedule.command).map(str::to_string));
    }
    for program in &programs {
        match tools::installed(&settings.shell, program).await {
            Ok(true) => tool_errors::Model::clear(&ctx.db, program, "").await?,
            Ok(false) => {
                let message = "It isn't installed: the login shell of m8r's user finds no such \
                               command. A run that starts it fails.";
                tool_errors::Model::set(&ctx.db, program, "", message).await?;
            }
            Err(err) => {
                let message = format!("m8r couldn't look for it with {}: {err}", settings.shell);
                tool_errors::Model::set(&ctx.db, program, "", &message).await?;
            }
        }
    }
    watchlist_items::Model::refresh(&ctx.db, &settings.shell).await?;
    check_pages(ctx).await?;

    // An error about a tool that m8r no longer runs has nothing left to clear it.
    for error in tool_errors::Model::list(&ctx.db).await? {
        let wanted = programs.contains(&error.tool)
            || error.tool == PAGES
            || watchlist_items::Forge::TOOLS.contains(&error.tool.as_str());
        if !wanted {
            tool_errors::Model::clear(&ctx.db, &error.tool, &error.host).await?;
        }
    }
    Ok(())
}

/// See whether a run can reach m8r's API: a tick runs as the same user, on the same host. If
/// it can't, an agent can't change the watchlist, and nothing else would say so.
async fn check_pages(ctx: &AppContext) -> Result<()> {
    let server = &ctx.config.server;
    let address = (
        server.binding.as_str(),
        u16::try_from(server.port).unwrap_or_default(),
    );
    let connection = tokio::net::TcpStream::connect(address);
    let problem = match tokio::time::timeout(PAGES_LIMIT, connection).await {
        Ok(Ok(_)) => return Ok(tool_errors::Model::clear(&ctx.db, PAGES, "").await?),
        Ok(Err(err)) => err.to_string(),
        Err(_) => "no answer in time".to_string(),
    };
    let message = format!(
        "A run can't reach m8r's API at {}, so its agent can't change the watchlist: {problem}. \
         The pages are down, or a firewall keeps m8r's user from them.",
        settings::local_url(ctx)
    );
    Ok(tool_errors::Model::set(&ctx.db, PAGES, "", &message).await?)
}

/// Run one queued run from start to end, and record how it went.
async fn execute(ctx: &AppContext, settings: &Settings, run: runs::Model) -> Result<()> {
    let dir = std::path::absolute(settings.artifacts_dir(run.id))?;
    let (schedule, prompt) = match prepare(ctx, &run, &dir).await? {
        Ok(ready) => ready,
        Err(why) => {
            run.into_active_model()
                .finished(&ctx.db, runs::Status::Failed, String::new(), why)
                .await?;
            return Ok(());
        }
    };
    fs::create_dir_all(&dir)?;
    let run = run
        .into_active_model()
        .started(&ctx.db, &schedule, prompt.clone())
        .await?;

    // What m8r's skill tells the agent to use (see `prompts`). The token ends with the run.
    let limit = settings.run_limit();
    let env = [
        ("M8R_URL", settings::local_url(ctx)),
        (
            "M8R_TOKEN",
            access::run_token(ctx, run.id, limit.as_secs() + 60)?,
        ),
    ];
    let printed = dir.with_extension("out");
    let end = tools::run_command(
        &settings.shell,
        &run.command,
        Path::new(&run.repo),
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
    // Whatever happened, keep what the run got done.
    artifacts::Model::collect(&ctx.db, run.id, &dir).await?;
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

/// The run's schedule as it is now, and its compiled prompt, which the command gets on stdin.
/// Or why the run can't start.
async fn prepare(
    ctx: &AppContext,
    run: &runs::Model,
    artifacts_dir: &Path,
) -> Result<Checked<(schedules::Model, String)>> {
    let Some(schedule_id) = run.schedule_id else {
        return Ok(Err(
            "The run's schedule was removed before the run started.".to_string(),
        ));
    };
    let (schedule, category) = schedules::Model::find(&ctx.db, schedule_id).await?;
    if configurations::Model::active(&ctx.db).await?.is_none() {
        return Ok(Err("No configuration is active.".to_string()));
    }
    if !Path::new(&schedule.repo).is_dir() {
        return Ok(Err(format!("{} isn't a directory.", schedule.repo)));
    }
    let artifacts_dir = artifacts_dir.to_string_lossy();
    let context = prompts::Context::load(&ctx.db, category.as_ref(), &artifacts_dir).await?;
    Ok(context
        .compile(&schedule.prompt)
        .map(|prompt| (schedule, prompt)))
}

/// The start of what the run printed: all of it, unless it's huge.
fn read_start(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = File::open(path) {
        let _ = file.take(OUTPUT_BYTES).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
