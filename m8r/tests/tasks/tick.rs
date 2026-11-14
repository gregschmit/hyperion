use std::fs::{self, File};

use loco_rs::{boot::run_task, prelude::*, task};
use m8r::{
    app::App,
    models::{
        artifacts,
        configurations::{self, Operation, Params},
        items, runs, schedules, tool_errors,
    },
    settings::Settings,
};
use serial_test::serial;

use crate::support::{Sandbox, GH};

async fn tick(ctx: &AppContext) {
    run_task::<App>(ctx, Some(&"tick".to_string()), &task::Vars::default())
        .await
        .unwrap();
}

/// An active configuration with this triage prompt, and no other.
async fn configure(ctx: &AppContext, triage_prompt: &str) {
    let params = Params {
        name: "Test".to_string(),
        triage_prompt: triage_prompt.to_string(),
        ..Params::starter()
    };
    configurations::Model::create(&ctx.db, &params)
        .await
        .unwrap()
        .into_active_model()
        .activated(&ctx.db)
        .await
        .unwrap();
}

async fn queue(ctx: &AppContext, operation: Operation, sandbox: &Sandbox) -> runs::Model {
    let repo = sandbox.path("repo");
    fs::create_dir_all(&repo).unwrap();
    runs::Model::queue(&ctx.db, operation, &repo.to_string_lossy())
        .await
        .unwrap()
        .unwrap()
}

async fn errors(ctx: &AppContext) -> Vec<(String, String)> {
    tool_errors::Model::list(&ctx.db)
        .await
        .unwrap()
        .into_iter()
        .map(|error| (error.tool, error.host))
        .collect()
}

/// An agent that reports as the system message says: a page, a link, and a loose file. Then
/// it does what `$1` says.
const REPORTER: &str = r#"
prompt=$(cat)
dir=$(printf '%s\n' "$prompt" | sed -n 's/^\(\/.*\/artifacts\/[0-9]*\), named.*/\1/p')
echo "working in $(pwd)"
mkdir -p "$dir/review of rxg!1" "$dir/comment"
echo "<h1>Review</h1>" > "$dir/review of rxg!1/index.html"
echo "https://git.example.com/g/p/-/merge_requests/1#note_5" > "$dir/comment/link.txt"
echo "loose" > "$dir/notes.md"
echo "done" >&2
exit "${1:-0}"
"#;

#[tokio::test]
#[serial]
async fn a_tick_runs_the_oldest_queued_run_with_its_compiled_prompt() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-run");
    let ctx = sandbox.context(&boot.app_context, "cat");
    configure(&ctx, "{{ system_message }}\nTriage this repository.").await;
    let run = queue(&ctx, Operation::Triage, &sandbox).await;
    let later = runs::Model::queue(&ctx.db, Operation::Triage, "/elsewhere")
        .await
        .unwrap()
        .unwrap();

    tick(&ctx).await;

    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.status, "ok", "{}", run.error);
    assert!(run.started_at.is_some() && run.finished_at.is_some());
    // `cat` prints its prompt back.
    assert_eq!(run.output, run.prompt);
    assert!(
        run.prompt.starts_with("You are running in m8r"),
        "{}",
        run.prompt
    );
    assert!(
        run.prompt.ends_with("\nTriage this repository."),
        "{}",
        run.prompt
    );
    // The agent is told the full path of the run's own artifact directory.
    let dir = Settings::from_context(&ctx).unwrap().artifacts_dir(run.id);
    assert!(dir.is_absolute() && dir.is_dir());
    assert!(
        run.prompt.contains(&format!("{}, named", dir.display())),
        "{}",
        run.prompt
    );
    // One run for each tick: the next one waits for the next tick.
    assert_eq!(
        runs::Model::find(&ctx.db, later.id).await.unwrap().status,
        "queued"
    );
    assert!(errors(&ctx).await.is_empty());
}

#[tokio::test]
#[serial]
async fn a_run_reports_artifacts_even_when_the_agent_fails() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-artifacts");
    sandbox.tool("reporter", REPORTER);
    let ctx = sandbox.context(&boot.app_context, "reporter 3");
    configure(&ctx, "{{ system_message }}").await;
    let run = queue(&ctx, Operation::Triage, &sandbox).await;

    tick(&ctx).await;

    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.status, "failed");
    assert_eq!(run.error, "the agent exited with status 3");
    // What it printed, stdout and stderr together. It ran in the repo.
    let repo = fs::canonicalize(sandbox.path("repo")).unwrap();
    assert_eq!(run.output, format!("working in {}\ndone\n", repo.display()));
    let reported: Vec<_> = artifacts::Model::of_run(&ctx.db, run.id)
        .await
        .unwrap()
        .into_iter()
        .map(|artifact| (artifact.name, artifact.url))
        .collect();
    assert_eq!(
        reported,
        [
            (
                "comment".to_string(),
                "https://git.example.com/g/p/-/merge_requests/1#note_5".to_string()
            ),
            ("report".to_string(), String::new()),
            ("review of rxg!1".to_string(), String::new()),
        ]
    );
}

#[tokio::test]
#[serial]
async fn a_run_fails_at_once_when_it_cannot_start() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-cannot-start");
    let ctx = sandbox.context(&boot.app_context, "cat");

    // No configuration is active.
    let run = queue(&ctx, Operation::Triage, &sandbox).await;
    tick(&ctx).await;
    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(
        (run.status.as_str(), run.error.as_str()),
        ("failed", "No configuration is active.")
    );
    assert!(run.started_at.is_none());

    // The configuration has no prompt for the operation.
    configure(&ctx, "{{ system_message }}").await;
    let run = queue(&ctx, Operation::Review, &sandbox).await;
    tick(&ctx).await;
    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.error, "The active configuration has no review prompt.");

    // The clone is gone.
    let run = runs::Model::queue(&ctx.db, Operation::Triage, "/no/such/clone")
        .await
        .unwrap()
        .unwrap();
    tick(&ctx).await;
    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.error, "/no/such/clone isn't a directory.");
}

#[tokio::test]
#[serial]
async fn a_tick_queues_the_schedules_that_are_due_once_configured() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-schedules");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let repo = sandbox.path("repo");
    fs::create_dir_all(&repo).unwrap();
    for (operation, cron) in [("triage", "* * * * *"), ("review", "0 0 31 2 *")] {
        let params = schedules::Params {
            operation: operation.to_string(),
            repo: repo.to_string_lossy().into_owned(),
            cron: cron.to_string(),
        };
        schedules::Model::create(&ctx.db, &params).await.unwrap();
    }

    // Unconfigured, m8r is idle.
    tick(&ctx).await;
    assert!(runs::Model::recent(&ctx.db, 10).await.unwrap().is_empty());

    configure(&ctx, "{{ system_message }}").await;
    tick(&ctx).await;
    let queued = runs::Model::recent(&ctx.db, 10).await.unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        (queued[0].operation.as_str(), queued[0].status.as_str()),
        ("triage", "ok")
    );
}

#[tokio::test]
#[serial]
async fn a_missing_agent_is_a_tool_error_until_it_shows_up() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-missing-agent");
    let ctx = sandbox.context(&boot.app_context, "claude -p");
    configure(&ctx, "{{ system_message }}").await;
    let run = queue(&ctx, Operation::Triage, &sandbox).await;

    tick(&ctx).await;
    assert_eq!(errors(&ctx).await, [("claude".to_string(), String::new())]);
    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.status, "failed");
    assert_eq!(
        run.error,
        "the agent command wasn't found (exit status 127)"
    );

    // The developer installs it. The next tick sees it.
    sandbox.tool("claude", "cat");
    tick(&ctx).await;
    assert!(errors(&ctx).await.is_empty());
}

#[tokio::test]
#[serial]
async fn a_tick_asks_the_forges_about_the_items() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-items");
    let ctx = sandbox.context(&boot.app_context, "cat");
    for url in [
        "https://github.com/o/r/issues/1",
        "https://github.com/o/r/issues/2",
    ] {
        let params = items::Params {
            url: url.to_string(),
            priority: "high".to_string(),
            note: String::new(),
        };
        items::Model::create(&ctx.db, &params).await.unwrap();
    }

    // gh isn't installed: an error, and the items wait.
    tick(&ctx).await;
    assert_eq!(errors(&ctx).await, [("gh".to_string(), String::new())]);
    assert_eq!(items::Model::list(&ctx.db).await.unwrap().len(), 2);

    // It shows up: the open item gets its title, and the closed one leaves.
    sandbox.tool("gh", GH);
    tick(&ctx).await;
    assert!(errors(&ctx).await.is_empty());
    let left = items::Model::list(&ctx.db).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].title, "An open one");
}

#[tokio::test]
#[serial]
async fn the_pages_say_when_the_scheduler_does_not_tick() {
    use loco_rs::controller::views::engines::TeraView;
    use m8r::views;

    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-heartbeat");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let engine = TeraView::build().unwrap();
    let page = |ctx: AppContext, engine: TeraView| async move {
        let res = views::page(
            &engine,
            &ctx,
            "runs/index.html",
            "runs",
            serde_json::json!({ "runs": [] }),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    };

    let before = page(ctx.clone(), engine.clone()).await;
    assert!(before.contains("The scheduler isn't running"), "{before}");
    assert!(before.contains("no tick yet"), "{before}");

    tick(&ctx).await;
    let after = page(ctx.clone(), engine).await;
    assert!(!after.contains("The scheduler isn't running"), "{after}");
    assert!(!after.contains("What doesn't work"), "{after}");
}

#[tokio::test]
#[serial]
async fn one_run_at_a_time() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-lock");
    let ctx = sandbox.context(&boot.app_context, "cat");
    configure(&ctx, "{{ system_message }}").await;
    let settings = Settings::from_context(&ctx).unwrap();
    fs::create_dir_all(&settings.data).unwrap();

    // Another tick has a run in progress: it holds the lock, and its run is marked running.
    let other = File::create(settings.lock_file()).unwrap();
    other.lock().unwrap();
    let in_progress = queue(&ctx, Operation::Develop, &sandbox).await;
    let in_progress = in_progress
        .into_active_model()
        .started(&ctx.db, "p".to_string())
        .await
        .unwrap();
    let waiting = queue(&ctx, Operation::Triage, &sandbox).await;

    tick(&ctx).await;
    assert_eq!(
        runs::Model::find(&ctx.db, waiting.id).await.unwrap().status,
        "queued"
    );
    assert_eq!(
        runs::Model::find(&ctx.db, in_progress.id)
            .await
            .unwrap()
            .status,
        "running"
    );

    // That tick dies, and the lock goes with it. The next tick fails its run, and moves on.
    drop(other);
    tick(&ctx).await;
    let cut_off = runs::Model::find(&ctx.db, in_progress.id).await.unwrap();
    assert_eq!(cut_off.status, "failed");
    assert_eq!(cut_off.error, "m8r stopped while the run was in progress");
    assert_eq!(
        runs::Model::find(&ctx.db, waiting.id).await.unwrap().status,
        "ok"
    );
}
