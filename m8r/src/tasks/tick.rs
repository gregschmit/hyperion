use std::{
    fs::{self, File, TryLockError},
    io::Read,
    path::Path,
};

use chrono::Local;
use loco_rs::prelude::*;

use crate::{
    models::{
        artifacts,
        configurations::{self, Operation},
        items, runs, schedules, tool_errors,
    },
    settings::Settings,
    tools,
};

/// How much of what an agent printed a run keeps.
const OUTPUT_BYTES: u64 = 1024 * 1024;

/// The scheduler runs this every minute, each time in a process of its own. m8r's schedules
/// are rows, not scheduler jobs, so this is where they're read.
pub struct Tick;
#[async_trait]
impl Task for Tick {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "tick".to_string(),
            detail: "Queue the schedules that are due, check the tools, and start the oldest \
                     queued run. The scheduler runs this every minute."
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
    let now = Local::now();
    for schedule in schedules::Model::list(&ctx.db).await? {
        let Some(operation) = Operation::parse(&schedule.operation) else {
            continue;
        };
        if schedule.is_due(&now) {
            runs::Model::queue(&ctx.db, operation, &schedule.repo).await?;
        }
    }
    Ok(())
}

/// See whether the tools work: the agent's program, and `gh` and `glab` for the items. A tool
/// that's missing or logged out is recorded, not fatal, and the record goes when it works.
async fn check_tools(ctx: &AppContext, settings: &Settings) -> Result<()> {
    let agent = settings.agent_program();
    match tools::installed(&settings.shell, agent).await {
        Ok(true) => tool_errors::Model::clear(&ctx.db, agent, "").await?,
        Ok(false) => {
            let message = "It isn't installed: the login shell of m8r's user finds no such \
                           command. Runs can't start without it.";
            tool_errors::Model::set(&ctx.db, agent, "", message).await?;
        }
        Err(err) => {
            let message = format!("m8r couldn't look for it with {}: {err}", settings.shell);
            tool_errors::Model::set(&ctx.db, agent, "", &message).await?;
        }
    }
    items::Model::refresh(&ctx.db, &settings.shell).await?;

    // An error about a tool that m8r no longer runs has nothing left to clear it.
    for error in tool_errors::Model::list(&ctx.db).await? {
        if error.tool != agent && !items::Forge::TOOLS.contains(&error.tool.as_str()) {
            tool_errors::Model::clear(&ctx.db, &error.tool, &error.host).await?;
        }
    }
    Ok(())
}

/// Run one queued run from start to end, and record how it went.
async fn execute(ctx: &AppContext, settings: &Settings, run: runs::Model) -> Result<()> {
    let dir = std::path::absolute(settings.artifacts_dir(run.id))?;
    let prompt = match prompt(ctx, &run, &dir).await? {
        Ok(prompt) => prompt,
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
        .started(&ctx.db, prompt.clone())
        .await?;

    let printed = dir.with_extension("out");
    let end = tools::agent(
        &settings.shell,
        &settings.agent,
        Path::new(&run.repo),
        &prompt,
        &printed,
        settings.run_limit(),
    )
    .await;
    let output = read_start(&printed);
    let _ = fs::remove_file(&printed);

    let problem = match end {
        Ok(end) => end.problem(settings.run_limit()),
        Err(err) => Some(format!("m8r couldn't start the agent: {err}")),
    };
    // Whatever happened, keep what the agent got done.
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

/// The run's compiled prompt, or why it can't start.
async fn prompt(
    ctx: &AppContext,
    run: &runs::Model,
    artifacts_dir: &Path,
) -> Result<std::result::Result<String, String>> {
    let Some(operation) = Operation::parse(&run.operation) else {
        return Ok(Err(format!(
            "m8r has no operation called {}.",
            run.operation
        )));
    };
    let Some(configuration) = configurations::Model::active(&ctx.db).await? else {
        return Ok(Err("No configuration is active.".to_string()));
    };
    if !Path::new(&run.repo).is_dir() {
        return Ok(Err(format!("{} isn't a directory.", run.repo)));
    }
    let items = items::Model::list(&ctx.db).await?;
    let compiled = configuration.compile(operation, &items, &artifacts_dir.to_string_lossy());
    Ok(match compiled {
        Ok(prompt) if prompt.trim().is_empty() => Err(format!(
            "The active configuration has no {} prompt.",
            operation.as_str()
        )),
        other => other,
    })
}

/// The start of what the agent printed: all of it, unless it's huge.
fn read_start(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = File::open(path) {
        let _ = file.take(OUTPUT_BYTES).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
