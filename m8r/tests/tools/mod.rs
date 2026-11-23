use std::{
    fs,
    time::{Duration, Instant},
};

use m8r::tools::{self, End};

use crate::support::Sandbox;

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

#[tokio::test]
async fn a_tool_gets_its_arguments_as_they_are() {
    let sandbox = Sandbox::new("tools-run");
    sandbox.tool("show", r#"for word in "$@"; do echo "[$word]"; done"#);

    let output = tools::run(
        &sandbox.shell(),
        &argv(&["show", "a b", "$(id)", "; rm -rf /", "%2F"]),
        tools::ANSWER_LIMIT,
    )
    .await
    .unwrap();

    assert!(output.ok());
    assert_eq!(output.stdout, "[a b]\n[$(id)]\n[; rm -rf /]\n[%2F]\n");
}

#[tokio::test]
async fn a_tool_gets_nothing_of_m8rs_own_environment() {
    let sandbox = Sandbox::new("tools-environment");
    // Set for this process as the units set them. No other test reads these: the names are
    // not the real ones, which a test's own database goes by. A tool takes a name that it
    // knows for its own setting, as Rails takes DATABASE_URL, so it gets none of them.
    std::env::set_var("M8R_SECRET", "hunter2");
    std::env::set_var("M8R_TEST_DATABASE_URL", "sqlite:///var/lib/m8r/m8r.sqlite");
    std::env::set_var("M8R_TEST_LOCO_ENV", "production");
    let home = std::env::var("HOME").unwrap();
    let own = ["hunter2", "M8R_SECRET", "M8R_TEST_", "m8r.sqlite", "CARGO"];

    let output = tools::run(&sandbox.shell(), &argv(&["env"]), tools::ANSWER_LIMIT)
        .await
        .unwrap();
    for name in own {
        assert!(!output.stdout.contains(name), "{name}: {}", output.stdout);
    }
    // It gets who the user is, which a login starts from.
    assert!(
        output.stdout.contains(&format!("HOME={home}\n")),
        "{}",
        output.stdout
    );

    // A run's command gets the same, and what m8r gives it for the API.
    let printed = sandbox.path("printed");
    tools::run_command(
        &sandbox.shell(),
        "env",
        &sandbox.dir,
        "",
        &printed,
        Duration::from_secs(30),
        &[("M8R_URL", "http://127.0.0.1:5150".to_string())],
    )
    .await
    .unwrap();
    let printed = fs::read_to_string(&printed).unwrap();
    for name in own {
        assert!(!printed.contains(name), "{name}: {printed}");
    }
    assert!(printed.contains(&format!("HOME={home}\n")), "{printed}");
    assert!(
        printed.contains("M8R_URL=http://127.0.0.1:5150\n"),
        "{printed}"
    );
}

#[tokio::test]
async fn a_tool_that_is_not_installed_is_missing() {
    let sandbox = Sandbox::new("tools-missing");
    let shell = sandbox.shell();

    let output = tools::run(&shell, &argv(&["gh", "api"]), tools::ANSWER_LIMIT)
        .await
        .unwrap();
    assert!(output.missing() && !output.ok());

    sandbox.tool("gh", "echo 'gh: Bad credentials (HTTP 401)' >&2\nexit 1");
    let output = tools::run(&shell, &argv(&["gh", "api"]), tools::ANSWER_LIMIT)
        .await
        .unwrap();
    assert!(!output.missing() && !output.ok());
    assert_eq!(output.complaint(), "gh: Bad credentials (HTTP 401)");

    // No shell at all is an error, not a missing tool.
    assert!(
        tools::run("/no/such/shell", &argv(&["gh"]), tools::ANSWER_LIMIT)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_tool_that_gives_no_answer_is_cut_off() {
    let sandbox = Sandbox::new("tools-slow");
    let started = Instant::now();

    let result = tools::run(
        &sandbox.shell(),
        &argv(&["sleep", "30"]),
        Duration::from_millis(200),
    )
    .await;

    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn an_agent_gets_the_prompt_on_stdin_and_runs_in_the_repo() {
    let sandbox = Sandbox::new("tools-agent");
    let repo = sandbox.path("repo");
    fs::create_dir_all(&repo).unwrap();
    let printed = sandbox.path("printed");

    let end = tools::run_command(
        &sandbox.shell(),
        "pwd; cat; echo ' (stderr)' >&2; exit 7",
        &repo,
        "the prompt",
        &printed,
        Duration::from_secs(30),
        &[],
    )
    .await
    .unwrap();

    assert_eq!(end, End::Exited(Some(7)));
    assert_eq!(
        end.problem(Duration::from_secs(60)).unwrap(),
        "the command exited with status 7"
    );
    let repo = fs::canonicalize(&repo).unwrap();
    assert_eq!(
        fs::read_to_string(&printed).unwrap(),
        format!("{}\nthe prompt (stderr)\n", repo.display())
    );
    assert_eq!(End::Exited(Some(0)).problem(Duration::from_secs(60)), None);
}

#[tokio::test]
async fn an_agent_that_passes_the_time_limit_is_killed_with_what_it_started() {
    let sandbox = Sandbox::new("tools-agent-limit");
    let child = sandbox.path("child-finished");
    let printed = sandbox.path("printed");
    // The agent starts a child of its own that would outlive it, and ignores a polite request.
    let command = format!(
        "trap '' TERM; (sleep 6; touch '{}') & echo started; sleep 30",
        child.display()
    );
    let started = Instant::now();

    let limit = Duration::from_millis(1500);
    let end = tools::run_command(
        &sandbox.shell(),
        &command,
        &sandbox.dir,
        "",
        &printed,
        limit,
        &[],
    )
    .await
    .unwrap();

    assert_eq!(end, End::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(6));
    assert_eq!(
        end.problem(Duration::from_secs(120)).unwrap(),
        "the run passed the time limit of 2 minutes"
    );
    assert_eq!(fs::read_to_string(&printed).unwrap(), "started\n");
    // Its child went with it: it never gets to the end of its sleep.
    tokio::time::sleep(Duration::from_secs(8).saturating_sub(started.elapsed())).await;
    assert!(!child.exists());
}
