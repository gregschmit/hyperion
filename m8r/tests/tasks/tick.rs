use std::fs::{self, File};

use loco_rs::{boot::run_task, prelude::*, task};
use m8r::{
    app::App,
    models::{artifacts, errors, runs, schedules, watchlist_items},
    settings::Settings,
};
use serial_test::serial;

use crate::support::{
    self, queue_all, queue_one, schedule, schedule_params, start, watch, Sandbox, GH, REQUESTS,
};

async fn tick(ctx: &AppContext) {
    run_task::<App>(ctx, Some(&"tick".to_string()), &task::Vars::default())
        .await
        .unwrap();
}

/// Queue a run of a new schedule in the sandbox's repo: the agent's command, with `prompt`.
/// The first one is called `triage`, and each later one has a name of its own.
async fn queue(ctx: &AppContext, sandbox: &Sandbox, prompt: &str) -> runs::Model {
    let made = schedules::Entity::find().count(&ctx.db).await.unwrap();
    let name = match made {
        0 => "triage".to_string(),
        made => format!("triage {}", made + 1),
    };
    let params = schedules::Params {
        name,
        command: Settings::from_context(ctx).unwrap().agent,
        ..schedule_params(&sandbox.repo(), prompt)
    };
    let schedule = schedule(ctx, &params).await;
    queue_one(&ctx.db, &schedule).await
}

/// The clone at `dir` as one of m8r's repos, with the project at `url`.
async fn repo(ctx: &AppContext, dir: &std::path::Path, url: &str) -> m8r::models::repos::Model {
    support::repo(&ctx.db, &dir.to_string_lossy(), url).await
}

/// What the open errors are about.
async fn errors(ctx: &AppContext) -> Vec<String> {
    let open = errors::Model::open(&ctx.db).await.unwrap();
    open.into_iter().map(|error| error.subject).collect()
}

/// An agent that reports as m8r's prompt says: a page, a link, and a loose file. Then it does
/// what `$1` says.
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
    // The forges say that everything on the watchlist is open.
    sandbox.tool("gh", r#"echo '{"title": "An open one", "state": "open"}'"#);
    sandbox.tool(
        "glab",
        r#"echo '{"title": "An open one", "state": "opened"}'"#,
    );
    let ctx = sandbox.context(&boot.app_context, "cat");
    // The run's repo has an issue and a PR, and another repo has an MR.
    repo(&ctx, &sandbox.repo(), "https://github.com/o/r").await;
    repo(
        &ctx,
        &sandbox.another_repo("other"),
        "https://git.example.com/g/p",
    )
    .await;
    watch(&ctx, "https://github.com/o/r/issues/1").await;
    watch(&ctx, "https://github.com/o/r/pull/2").await;
    watch(&ctx, "https://git.example.com/g/p/-/merge_requests/3").await;
    // A review of the PRs/MRs: the schedule's category says which of its repo's items its
    // prompt gets.
    let params = schedules::Params {
        name: "review".to_string(),
        category: REQUESTS.to_string(),
        ..schedule_params(
            &sandbox.repo(),
            "{{ m8r_prompt }}\n\nReview these:\n{{ watchlist }}",
        )
    };
    let reviews = schedule(&ctx, &params).await;
    let run = queue_one(&ctx.db, &reviews).await;
    let elsewhere = sandbox.another_repo("elsewhere");
    let later = support::queue(&ctx.db, "triage", &elsewhere.to_string_lossy(), "cat").await;

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
        run.prompt.ends_with(
            "they are the developer's.\n\n\
             Review these:\n\
             - item 2 (medium): https://github.com/o/r/pull/2"
        ),
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
    // One schedule for each tick: the next one's run waits for the next tick.
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
    let run = queue(&ctx, &sandbox, "{{ m8r_prompt }}").await;

    tick(&ctx).await;

    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.status, "failed");
    assert_eq!(run.error, "the command exited with status 3");
    // What it printed, stdout and stderr together. It ran in the repo.
    let repo = fs::canonicalize(sandbox.repo()).unwrap();
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

/// An agent that keeps the watchlist as m8r's prompt says, with the prompt's own commands: it
/// takes each one from its prompt, puts in what the prompt leaves open, and runs it. Then it
/// tries what its token doesn't cover, and leaves the token for the test.
const KEEPER: &str = r#"
prompt=$(cat)
# The lines of the prompt's command that has `$1` in its first line.
given() { printf '%s\n' "$prompt" | grep -A1 -- "$1" | grep -v '^--$' | sed -e '/[^\\]$/q'; }

echo "listed: $(eval "$(given 'api/watchlist_items?repo_id=')")"
# The PR of item 1 merged, and the review of item 2 is done.
eval "$(given '-X DELETE' | sed 's/NUMBER/1/')" > /dev/null
eval "$(given '-X PATCH' | sed 's/NUMBER/2/')" > /dev/null
# The agent opened a PR for the developer to follow, and one in a repo that m8r doesn't have.
add=$(given 'watchlist_items" -H "Authorization: Bearer $M8R_TOKEN" \\$')
added() { printf '%s\n' "$add" | sed -e "s|\"URL\"|\"$1\"|"; }
eval "$(added https://github.com/o/r/pull/9)" > /dev/null
echo "elsewhere: $(eval "$(added https://github.com/o/unknown/pull/1)")"

echo "schedules: $(curl -sS -o /dev/null -w '%{http_code}' "$M8R_URL/api/schedules" -H "Authorization: Bearer $M8R_TOKEN")"
printf '%s %s' "$M8R_URL" "$M8R_TOKEN" > left-by-the-run
"#;

#[tokio::test]
#[serial]
async fn a_runs_agent_keeps_the_watchlist_with_the_commands_of_m8rs_prompt() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-watchlist-api");
    sandbox.tool("keeper", KEEPER);
    // The pages are up, where the run's environment says.
    sandbox.serve(boot.router.clone().unwrap());
    let ctx = sandbox.context(&boot.app_context, "keeper");
    // The run's repo has two PRs, and another repo has an MR.
    repo(&ctx, &sandbox.repo(), "https://github.com/o/r").await;
    let merged = watch(&ctx, "https://github.com/o/r/pull/1").await;
    let review = watchlist_items::Params {
        url: "https://github.com/o/r/pull/2".to_string(),
        requested: true,
        ..watchlist_items::Params::default()
    };
    watchlist_items::Model::create(&ctx.db, &review)
        .await
        .unwrap()
        .unwrap();
    let other = watch(&ctx, "https://git.example.com/g/p/-/merge_requests/3").await;
    let run = queue(&ctx, &sandbox, "{{ m8r_prompt }}").await;

    tick(&ctx).await;

    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.status, "ok", "{} {}", run.error, run.output);
    // It read the list of its own repo: the requested item first, and no other repo's.
    assert!(
        run.output
            .starts_with(r#"listed: [{"category":"pull/merge requests","checked_at":null,"id":2,"#),
        "{}",
        run.output
    );
    assert!(!run.output.contains(&other.url), "{}", run.output);
    // m8r takes an item only for a repo that it has, and says so.
    assert!(
        run.output.contains(
            "elsewhere: {\"error\":\"Bad Request\",\"description\":\"m8r has no repo for \
             https://github.com/o/unknown. Add that repo first: an item is in one.\"}"
        ),
        "{}",
        run.output
    );
    // Its token covers the watchlist alone.
    assert!(run.output.ends_with("schedules: 403\n"), "{}", run.output);
    // The prompt, which the run's page shows, holds neither the address nor the token.
    assert!(!run.prompt.contains("http://127.0.0.1"), "{}", run.prompt);
    assert!(!run.prompt.contains("eyJ"), "{}", run.prompt);

    let left: Vec<_> = watchlist_items::Model::list(&ctx.db, watchlist_items::Filter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|(item, repo)| (item.url, item.requested, item.note, repo.url))
        .collect();
    assert!(!left.iter().any(|item| item.0 == merged.url));
    let item = |url: &str, note: &str, project: &str| {
        (
            url.to_string(),
            false,
            note.to_string(),
            project.to_string(),
        )
    };
    assert_eq!(
        left,
        [
            item(
                "https://github.com/o/r/pull/2",
                "",
                "https://github.com/o/r"
            ),
            item(
                "https://github.com/o/r/pull/9",
                "Why the developer follows it",
                "https://github.com/o/r"
            ),
            item(&other.url, "", "https://git.example.com/g/p"),
        ]
    );

    // The run is over, and so is its token.
    let left = fs::read_to_string(sandbox.repo().join("left-by-the-run")).unwrap();
    let (url, token) = left.split_once(' ').unwrap();
    // Not std's Command: the test's own thread serves the pages.
    let status = tokio::process::Command::new("curl")
        .args(["-sS", "-o", "/dev/null", "-w", "%{http_code}"])
        .args([&format!("{url}/api/watchlist_items"), "-H"])
        .arg(format!("Authorization: Bearer {token}"))
        .output()
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), "401");
}

#[tokio::test]
#[serial]
async fn a_run_fails_at_once_when_it_cannot_start() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-cannot-start");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let failed = |ctx: AppContext, run: runs::Model| async move {
        tick(&ctx).await;
        let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
        assert_eq!(run.status, "failed");
        assert!(run.started_at.is_none());
        run.error
    };

    // The schedule went while its run waited.
    let run = queue(&ctx, &sandbox, "Triage.").await;
    schedules::Model::remove(&ctx.db, run.schedule_id.unwrap())
        .await
        .unwrap();
    assert_eq!(
        failed(ctx.clone(), run).await,
        "The run's schedule was removed before the run started."
    );

    // The clone is gone.
    let run = support::queue(&ctx.db, "gone clone", "/no/such/clone", "cat").await;
    assert_eq!(
        failed(ctx.clone(), run).await,
        "/no/such/clone isn't a directory."
    );

    // The repo went while its run waited.
    let run = queue(&ctx, &sandbox, "Triage.").await;
    let mut nowhere = run.clone().into_active_model();
    nowhere.repo_id = Set(None);
    nowhere.update(&ctx.db).await.unwrap();
    assert_eq!(
        failed(ctx.clone(), run).await,
        "The run's repo was removed before the run started."
    );

    // The prompt doesn't compile. m8r checks one when it saves it, so this one came another
    // way.
    let run = queue(&ctx, &sandbox, "Triage.").await;
    let found = schedules::Model::find(&ctx.db, run.schedule_id.unwrap()).await;
    let mut broken = found.unwrap().0.into_active_model();
    broken.prompt = Set("{{ gone }}".to_string());
    broken.update(&ctx.db).await.unwrap();
    let error = failed(ctx.clone(), run).await;
    assert!(error.starts_with("The prompt doesn't compile."), "{error}");
}

#[tokio::test]
#[serial]
async fn a_tick_queues_the_schedules_that_are_due() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-schedules");
    let ctx = sandbox.context(&boot.app_context, "cat");
    // One that's due every minute, and one whose only time is long gone.
    for (name, hron) in [
        ("triage", "every minute"),
        ("review", "on 2001-01-01 at 00:00"),
    ] {
        let params = schedules::Params {
            name: name.to_string(),
            hron: hron.into(),
            ..schedule_params(&sandbox.repo(), "Go.")
        };
        schedule(&ctx, &params).await;
    }

    tick(&ctx).await;
    let queued = runs::Model::recent(&ctx.db, 10).await.unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        (queued[0].name.as_str(), queued[0].status.as_str()),
        ("triage", "ok")
    );
    assert_eq!(queued[0].output, "Go.");
}

#[tokio::test]
#[serial]
async fn a_schedules_error_lasts_until_its_next_run_goes_well() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-schedule-error");
    // The command isn't there yet: the developer still has to install it.
    let ctx = sandbox.context(&boot.app_context, "claude -p");
    let run = queue(&ctx, &sandbox, "Triage.").await;
    let (schedule, _) = schedules::Model::find(&ctx.db, run.schedule_id.unwrap())
        .await
        .unwrap();
    let repo = sandbox.repo();
    let open = |ctx: AppContext| async move { errors::Model::open(&ctx.db).await.unwrap() };

    tick(&ctx).await;
    let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
    assert_eq!(run.status, "failed");
    assert_eq!(run.error, "the command wasn't found (exit status 127)");
    // The run that failed raised the error of its schedule in its repo, which leads to the run.
    let raised = open(ctx.clone()).await;
    assert_eq!(raised.len(), 1);
    assert_eq!(
        raised[0].subject,
        format!("schedule {} in {}", schedule.id, repo.display())
    );
    assert_eq!(
        raised[0].message,
        format!(
            "Run {} (triage) failed: the command wasn't found (exit status 127)",
            run.id
        )
    );
    assert_eq!(raised[0].run_id, Some(run.id));

    // It fails again: still one error, which says what the latest run did.
    let again = queue_one(&ctx.db, &schedule).await;
    tick(&ctx).await;
    let still = open(ctx.clone()).await;
    assert_eq!(still.len(), 1);
    assert_eq!(still[0].id, raised[0].id);
    assert_eq!(still[0].run_id, Some(again.id));
    assert!(still[0].message.starts_with(&format!("Run {} ", again.id)));

    // The developer installs the command. The next run goes well, and that clears the error.
    sandbox.tool("claude", "cat");
    queue_all(&ctx.db, &schedule).await;
    tick(&ctx).await;
    assert!(open(ctx.clone()).await.is_empty());
    // The list keeps it.
    let kept = errors::Model::list(&ctx.db, 10).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].cleared_at.is_some());
}

#[tokio::test]
#[serial]
async fn a_tick_says_what_m8r_needs_before_anything_runs() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-setup");
    let ctx = sandbox.context(&boot.app_context, "cat");

    // A new m8r has no schedule.
    tick(&ctx).await;
    assert_eq!(errors(&ctx).await, ["schedules"]);

    // The error goes when there's one.
    let only = schedule(&ctx, &schedule_params(&sandbox.repo(), "Go.")).await;
    tick(&ctx).await;
    assert!(errors(&ctx).await.is_empty());

    // And it's back when that goes.
    schedules::Model::remove(&ctx.db, only.id).await.unwrap();
    tick(&ctx).await;
    assert_eq!(errors(&ctx).await, ["schedules"]);
}

/// A command that isn't an agent's: it says what it was started with and what it got.
const OTHER: &str = r#"
echo "arguments: $*"
echo "stdin: $(cat)"
echo "api: ${M8R_URL:+there} ${M8R_TOKEN:+there}"
"#;

#[tokio::test]
#[serial]
async fn a_run_is_its_schedules_command_with_its_prompt_on_stdin() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-other");
    sandbox.tool("other-tool", OTHER);
    // The agent's command is another one: a run uses its schedule's.
    let ctx = sandbox.context(&boot.app_context, "false");
    let command = r#"other-tool "Update the changelog.""#;
    let params = |prompt: &str| schedules::Params {
        name: "other".to_string(),
        hron: "every minute".into(),
        command: command.to_string(),
        ..schedule_params(&sandbox.repo(), prompt)
    };
    // A command that says what to do itself needs no prompt.
    let blank = schedule(&ctx, &params("")).await;

    tick(&ctx).await;

    let ran = runs::Model::recent(&ctx.db, 10).await.unwrap();
    assert_eq!(ran.len(), 1);
    assert_eq!(ran[0].status, "ok", "{}", ran[0].error);
    assert_eq!(ran[0].command, command);
    assert_eq!(ran[0].prompt, "");
    assert_eq!(
        ran[0].output,
        "arguments: Update the changelog.\nstdin: \napi: there there\n"
    );

    // And one with a prompt gets it. Its schedule has no category, so it gets no items,
    // whatever is on the watchlist.
    blank.delete(&ctx.db).await.unwrap();
    watch(&ctx, "https://github.com/o/r/issues/1").await;
    schedule(&ctx, &params("For [{{ category }}]: {{ watchlist }}")).await;
    tick(&ctx).await;
    let ran = runs::Model::recent(&ctx.db, 1).await.unwrap();
    assert_eq!(
        ran[0].output,
        "arguments: Update the changelog.\nstdin: For []: (nothing)\napi: there there\n"
    );
}

#[tokio::test]
#[serial]
async fn a_tick_asks_the_forges_about_the_watchlist() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-watchlist");
    let ctx = sandbox.context(&boot.app_context, "cat");
    for url in [
        "https://github.com/o/r/issues/1",
        "https://github.com/o/r/issues/2",
    ] {
        watch(&ctx, url).await;
    }
    let listed = |ctx: AppContext| async move {
        let all = watchlist_items::Filter::default();
        let listed = watchlist_items::Model::list(&ctx.db, all).await.unwrap();
        listed.into_iter().map(|(item, _)| item).collect::<Vec<_>>()
    };

    // gh isn't installed: an error, and the items wait.
    tick(&ctx).await;
    assert!(errors(&ctx).await.contains(&"gh".to_string()));
    assert_eq!(listed(ctx.clone()).await.len(), 2);

    // It shows up: the open issue gets its title, and the closed one leaves the watchlist.
    sandbox.tool("gh", GH);
    tick(&ctx).await;
    assert!(!errors(&ctx).await.contains(&"gh".to_string()));
    let left = listed(ctx.clone()).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].title, "An open one");
}

#[tokio::test]
#[serial]
async fn a_tick_says_when_a_run_could_not_reach_the_api() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-pages");
    let up = sandbox.context(&boot.app_context, "cat");
    // Nothing listens where the pages should be: they're down, or a firewall is in the way.
    let mut down = up.clone();
    down.config.server.port = sandbox.closed_port().into();

    tick(&down).await;
    let said = errors::Model::open(&down.db).await.unwrap();
    let said: Vec<_> = said.iter().filter(|error| error.subject == "API").collect();
    assert_eq!(said.len(), 1);
    assert!(
        said[0].message.starts_with(&format!(
            "A run can't reach m8r's API at http://127.0.0.1:{}, so its agent can't change \
             the watchlist:",
            down.config.server.port
        )),
        "{}",
        said[0].message
    );

    // They answer again, and the next tick sees it.
    tick(&up).await;
    assert!(!errors(&up).await.contains(&"API".to_string()));
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
            serde_json::json!({ "runs": [], "setup": false }),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    };

    let before = page(ctx.clone(), engine.clone()).await;
    assert!(
        before.contains("<strong>scheduler:</strong> The scheduler isn&#39;t running"),
        "{before}"
    );
    assert!(before.contains("no tick yet"), "{before}");

    // A tick clears the error that the page raised.
    assert!(errors(&ctx).await.contains(&"scheduler".to_string()));
    tick(&ctx).await;
    assert!(!errors(&ctx).await.contains(&"scheduler".to_string()));
    let after = page(ctx.clone(), engine).await;
    assert!(!after.contains("<strong>scheduler:</strong>"), "{after}");
}

#[tokio::test]
#[serial]
async fn one_run_at_a_time() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-lock");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let settings = Settings::from_context(&ctx).unwrap();
    fs::create_dir_all(&settings.data).unwrap();

    // Another tick has a run in progress: it holds the lock, and its run is marked running.
    let other = File::create(settings.lock_file()).unwrap();
    other.lock().unwrap();
    let in_progress = queue(&ctx, &sandbox, "Develop.").await;
    let in_progress = start(&ctx.db, in_progress, "p").await;
    let waiting = queue(&ctx, &sandbox, "Triage.").await;

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

/// A command that waits for company. It leaves its mark beside the clones, and ends well once
/// `$1` clones have left theirs. Alone, it gives up.
const MEET: &str = r#"
marks="$(dirname "$(pwd)")/marks"
mkdir -p "$marks"
touch "$marks/$(basename "$(pwd)")"
tries=0
until [ "$(ls "$marks" | wc -l)" -ge "$1" ]; do
  tries=$((tries + 1))
  [ "$tries" -gt 100 ] && { echo "alone in $(basename "$(pwd)")"; exit 1; }
  sleep 0.1
done
echo "met in $(basename "$(pwd)")"
"#;

#[tokio::test]
#[serial]
async fn a_schedules_runs_start_together_one_in_each_of_its_repos() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-together");
    sandbox.tool("meet", MEET);
    let ctx = sandbox.context(&boot.app_context, "cat");
    let clones = [
        sandbox.repo(),
        sandbox.another_repo("second"),
        sandbox.another_repo("third"),
    ];
    let repos: Vec<_> = clones.iter().map(|clone| clone.to_string_lossy()).collect();
    let params = schedules::Params {
        repos: repos.join("\n").as_str().into(),
        command: "meet 3".to_string(),
        ..schedule_params(&clones[0], "")
    };
    let everywhere = schedule(&ctx, &params).await;
    let queued = queue_all(&ctx.db, &everywhere).await;
    assert_eq!(queued.len(), 3);
    let elsewhere = sandbox.another_repo("elsewhere");
    let later = support::queue(&ctx.db, "later", &elsewhere.to_string_lossy(), "cat").await;

    tick(&ctx).await;

    // Each command ended well, and it could only because the others ran at the same time.
    for (run, repo) in queued.iter().zip(&repos) {
        let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
        assert_eq!(run.status, "ok", "{} {}", run.error, run.output);
        assert_eq!(&run.repo, repo);
        let clone = std::path::Path::new(&run.repo).file_name().unwrap();
        assert_eq!(run.output, format!("met in {}\n", clone.to_string_lossy()));
    }
    // The schedules take turns: another one's run waits for the next tick.
    assert_eq!(
        runs::Model::find(&ctx.db, later.id).await.unwrap().status,
        "queued"
    );
    assert!(errors(&ctx).await.is_empty());
}

/// A command that fails in a clone that holds a file named `broken`.
const PICKY: &str = r#"
[ -e broken ] && { echo "broken here"; exit 3; }
echo "fine here"
"#;

#[tokio::test]
#[serial]
async fn each_of_a_schedules_repos_has_an_error_of_its_own() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-repo-errors");
    sandbox.tool("picky", PICKY);
    let ctx = sandbox.context(&boot.app_context, "cat");
    let (good, bad) = (sandbox.repo(), sandbox.another_repo("second"));
    fs::write(bad.join("broken"), "").unwrap();
    let params = schedules::Params {
        repos: format!("{}\n{}", good.display(), bad.display())
            .as_str()
            .into(),
        command: "picky".to_string(),
        ..schedule_params(&good, "")
    };
    let both = schedule(&ctx, &params).await;
    let statuses = |ctx: AppContext, runs: Vec<runs::Model>| async move {
        let mut statuses = Vec::new();
        for run in runs {
            let run = runs::Model::find(&ctx.db, run.id).await.unwrap();
            statuses.push(run.status);
        }
        statuses
    };

    // In one tick, the run in one clone goes well and the run in the other fails. The one
    // that went well doesn't clear the other's error.
    let queued = queue_all(&ctx.db, &both).await;
    tick(&ctx).await;
    assert_eq!(
        statuses(ctx.clone(), queued.clone()).await,
        ["ok", "failed"]
    );
    let open = errors::Model::open(&ctx.db).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].subject,
        format!("schedule {} in {}", both.id, bad.display())
    );
    assert_eq!(open[0].run_id, Some(queued[1].id));
    assert_eq!(
        open[0].message,
        format!(
            "Run {} (triage) failed: the command exited with status 3",
            queued[1].id
        )
    );

    // That clone is put right, and its next run clears its error.
    fs::remove_file(bad.join("broken")).unwrap();
    let queued = queue_all(&ctx.db, &both).await;
    tick(&ctx).await;
    assert_eq!(statuses(ctx.clone(), queued).await, ["ok", "ok"]);
    assert!(errors(&ctx).await.is_empty());
}

#[tokio::test]
#[serial]
async fn each_run_gets_the_items_of_its_own_repo() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-repo-items");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let (one, two) = (sandbox.repo(), sandbox.another_repo("second"));
    repo(&ctx, &one, "https://github.com/o/r").await;
    repo(&ctx, &two, "https://git.example.com/g/p").await;
    // Three items in the first repo: the last one is urgent, and someone asked for the
    // second. One item in the second repo.
    for (url, priority, requested) in [
        ("https://github.com/o/r/issues/1", "medium", false),
        ("https://github.com/o/r/pull/2", "medium", true),
        ("https://github.com/o/r/issues/3", "high", false),
        (
            "https://git.example.com/g/p/-/merge_requests/4",
            "medium",
            false,
        ),
    ] {
        let params = watchlist_items::Params {
            url: url.to_string(),
            priority: priority.to_string(),
            requested,
            ..watchlist_items::Params::default()
        };
        let listed = watchlist_items::Model::create(&ctx.db, &params).await;
        listed.unwrap().unwrap();
    }
    // One schedule in both repos. It takes every item, and two at most.
    let params = schedules::Params {
        name: "review".to_string(),
        category: "all".to_string(),
        max_items: 2,
        repos: format!("{}\n{}", one.display(), two.display())
            .as_str()
            .into(),
        ..schedule_params(
            &one,
            "{{ repo_url }}\nFirst:\n{{ requested_watchlist }}\n\
             All, {{ max_items }} at most:\n{{ watchlist }}",
        )
    };
    let both = schedule(&ctx, &params).await;
    let queued = queue_all(&ctx.db, &both).await;

    tick(&ctx).await;

    // `cat` prints its prompt back. Each run got its own repo's items and no other's: what
    // someone asked for, then what's urgent, and the cutoff left the third one out.
    let in_one = runs::Model::find(&ctx.db, queued[0].id).await.unwrap();
    assert_eq!(
        in_one.output,
        "https://github.com/o/r\n\
         First:\n\
         - item 2 (medium, requested): https://github.com/o/r/pull/2\n\
         All, 2 at most:\n\
         - item 2 (medium, requested): https://github.com/o/r/pull/2\n\
         - item 3 (high): https://github.com/o/r/issues/3"
    );
    let in_two = runs::Model::find(&ctx.db, queued[1].id).await.unwrap();
    assert_eq!(
        in_two.output,
        "https://git.example.com/g/p\n\
         First:\n\
         (nothing)\n\
         All, 2 at most:\n\
         - item 4 (medium): https://git.example.com/g/p/-/merge_requests/4"
    );
}

#[tokio::test]
#[serial]
async fn a_tick_says_when_a_repos_clone_is_not_there() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("tick-repos");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let gone = sandbox.path("gone");
    let lost = repo(&ctx, &gone, "").await;
    repo(&ctx, &sandbox.repo(), "").await;

    tick(&ctx).await;
    let subject = format!("repo {}", lost.path);
    assert!(errors(&ctx).await.contains(&subject));

    // The clone comes, and the next tick sees it.
    fs::create_dir_all(&gone).unwrap();
    tick(&ctx).await;
    assert!(!errors(&ctx).await.iter().any(|it| it.starts_with("repo ")));
}
