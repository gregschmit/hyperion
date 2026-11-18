use std::path::Path;

use jiff::{civil::date, Zoned};
use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        errors,
        runs::{self, Status},
        schedules::{self, parse, Changes, Operation, Params},
    },
    prompts,
};
use serial_test::serial;

use crate::support::{configure, start, watch, ISSUE, PR};

fn params(operation: &str, repo: &str, hron: &str) -> Params {
    Params {
        operation: operation.to_string(),
        repos: repo.into(),
        hron: hron.into(),
        command: "claude -p".to_string(),
        ..Params::default()
    }
}

fn here() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// How m8r reads `text`, in hron's own words.
fn read(text: &str) -> String {
    parse(text).unwrap().to_string()
}

/// A time on a server in Chicago. 2026-11-16 is a Monday.
fn chicago(day: i8, hour: i8, minute: i8, second: i8) -> Zoned {
    date(2026, 11, day)
        .at(hour, minute, second, 0)
        .in_tz("America/Chicago")
        .unwrap()
}

#[test]
fn a_schedule_says_when_in_words() {
    assert_eq!(read("every weekday at 6:00"), "every weekday at 06:00");
    assert_eq!(
        read("  Every Monday, Thursday at 9:00, 15:00 "),
        "every monday, thursday at 09:00, 15:00"
    );
    assert_eq!(
        read("every 30 min from 09:00 to 17:00 on weekdays"),
        "every 30 min from 09:00 to 17:00 on weekday"
    );
    assert_eq!(
        read("every day at 09:00 in america/new_york"),
        "every day at 09:00 in America/New_York"
    );
}

#[test]
fn an_interval_with_no_window_means_the_whole_day() {
    assert_eq!(read("every 12 hours"), "every 12 hours from 00:00 to 23:59");
    assert_eq!(read("Every hour"), "every 1 hour from 00:00 to 23:59");
    assert_eq!(
        read("every 15 minutes on weekdays"),
        "every 15 min from 00:00 to 23:59 on weekday"
    );
    // A window that's there stays, and so does what isn't an interval.
    assert_eq!(
        read("every 2 hours from 08:00 to 18:00"),
        "every 2 hours from 08:00 to 18:00"
    );
    assert_eq!(read("every 2 days at 9:00"), "every 2 days at 09:00");
}

#[test]
fn a_cron_expression_is_put_in_words() {
    assert_eq!(read("0 6 * * 1-5"), "every weekday at 06:00");
    assert_eq!(read(" */15  * * * * "), "every 15 min from 00:00 to 23:59");
    assert_eq!(read("30 13 * * *"), "every day at 13:30");

    // Not Loco's own form, with seconds first.
    let problem = parse("0 0 6 * * 1-5").unwrap_err();
    assert_eq!(
        problem,
        "m8r can't use that cron expression: expected 5 cron fields, got 6."
    );
    assert!(parse("61 6 * * *").is_err());
}

#[test]
fn a_time_that_m8r_cannot_read_says_where_it_went_wrong() {
    let problem = parse("every day at 6am").unwrap_err();
    assert_eq!(
        problem,
        "error: unknown keyword 'am'\n  every day at 6am\n                ^^"
    );
    for text in ["", "whenever", "every", "at six"] {
        assert!(parse(text).is_err(), "{text}");
    }
}

#[test]
fn a_repo_is_a_path_from_the_home_of_m8rs_user() {
    let home = Path::new("/home/m8r");
    for (repo, dir) in [
        ("~/git/rgnets/rxg", "/home/m8r/git/rgnets/rxg"),
        (" ~/git/rgnets/rxg ", "/home/m8r/git/rgnets/rxg"),
        ("git/rgnets/rxg", "/home/m8r/git/rgnets/rxg"),
        ("~", "/home/m8r"),
        // A full path is that path.
        ("/srv/clones/rxg", "/srv/clones/rxg"),
        // Another user's home is no shorthand of m8r's.
        ("~gns/git/rxg", "/home/m8r/~gns/git/rxg"),
    ] {
        assert_eq!(schedules::from_home(repo, home), Path::new(dir), "{repo}");
    }

    // The home is the one of the user that m8r runs as.
    let home = std::env::home_dir().unwrap();
    assert_eq!(schedules::repo_dir("~/git/x"), home.join("git/x"));
}

#[tokio::test]
#[serial]
async fn a_schedule_takes_a_clone_by_its_path_from_home() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    // A clone that's there, written from the home directory.
    let home = std::env::home_dir().unwrap();
    let Ok(inside) = std::env::current_dir()
        .unwrap()
        .strip_prefix(&home)
        .map(Path::to_path_buf)
    else {
        // The tests don't run under the home directory here, so there's no such clone.
        return;
    };
    let repo = format!("~/{}", inside.display());

    let made = schedules::Model::create(db, &params("triage", &repo, "every day at 9:00")).await;
    let (schedule, _) = made.unwrap().unwrap();

    // It's kept as it was written.
    assert_eq!(schedule.repos, repo);
    assert_eq!(schedules::repo_dir(&schedule.repos), home.join(inside));
}

#[test]
fn a_new_schedule_starts_with_its_operations_command_and_m8rs_parts_in_its_prompt() {
    let starter = Params::starter("claude -p --model opus");
    assert_eq!(starter.operation, "triage");
    assert_eq!(starter.command, "claude -p --model opus");
    assert_eq!(starter.prompt, prompts::STARTER);
    assert!(starter.category.is_empty());

    // For other, you write the command.
    assert_eq!(Operation::Review.default_command("claude -p"), "claude -p");
    assert_eq!(Operation::Other.default_command("claude -p"), "");
    assert_eq!(Operation::parse("other"), Some(Operation::Other));
    assert_eq!(Operation::parse("deploy"), None);
}

#[tokio::test]
#[serial]
async fn a_schedule_needs_an_operation_a_clone_a_time_and_a_command() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let saved = |params: Params| async move {
        let made = schedules::Model::create(db, &params).await;
        made.unwrap()
            .map(|(schedule, category)| (schedule, category.map(|it| it.name)))
    };

    assert!(saved(params("triage", &here(), "0 6 * * 1-5"))
        .await
        .is_ok());
    assert!(saved(params("other", &here(), "every 12 hours"))
        .await
        .is_ok());

    assert!(saved(params("deploy", &here(), "0 6 * * 1-5"))
        .await
        .is_err());
    assert!(saved(params("triage", "/no/such/clone", "0 6 * * 1-5"))
        .await
        .is_err());
    assert!(saved(params("triage", "no/such/clone", "0 6 * * 1-5"))
        .await
        .is_err());
    assert_eq!(
        saved(params("triage", " ", "0 6 * * 1-5"))
            .await
            .unwrap_err(),
        "Say which repo the schedule runs in: a clone in the home of m8r's user, such as \
         ~/git/group/project."
    );
    assert_eq!(
        saved(params("triage", "no/such/clone", "0 6 * * 1-5"))
            .await
            .unwrap_err(),
        "A repo must be a clone in the home of m8r's user, such as ~/git/group/project, and \
         \"no/such/clone\" isn't a directory there."
    );
    assert!(saved(params("triage", &here(), "at six")).await.is_err());
    assert_eq!(
        saved(params("triage", &here(), " \n")).await.unwrap_err(),
        "Say when the schedule runs, such as every weekday at 06:00."
    );
    let no_command = Params {
        command: "  ".to_string(),
        ..params("triage", &here(), "0 6 * * 1-5")
    };
    assert_eq!(
        saved(no_command).await.unwrap_err(),
        "Say which command the schedule runs."
    );

    // Its category is one that's there, or none: then its runs get no items.
    let review = |category: &str| Params {
        category: category.to_string(),
        ..params("review", &here(), "every day at 9:00")
    };
    let (schedule, category) = saved(review(PR)).await.unwrap();
    assert!(schedule.category_id.is_some());
    assert_eq!(category.as_deref(), Some(PR));
    let (schedule, category) = saved(review(" ")).await.unwrap();
    assert_eq!((schedule.category_id, category), (None, None));
    assert_eq!(
        saved(review("Epic")).await.unwrap_err(),
        "There's no category \"Epic\". The categories are: Issue, Pull/Merge Request."
    );

    // Its prompt compiles with what a run would get.
    let prompted = |prompt: &str| Params {
        prompt: prompt.to_string(),
        ..params("triage", &here(), "every day at 9:00")
    };
    assert!(saved(prompted(prompts::STARTER)).await.is_ok());
    assert!(saved(prompted("")).await.is_ok());
    let broken = saved(prompted("{{ items }}")).await.unwrap_err();
    assert!(
        broken.starts_with("The prompt doesn't compile."),
        "{broken}"
    );
    assert_eq!(schedules::Model::list(db).await.unwrap().len(), 6);
}

#[tokio::test]
#[serial]
async fn a_change_to_a_schedule_keeps_what_it_does_not_name() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    watch(ctx, "https://github.com/o/r/issues/1", ISSUE).await;
    let asked = Params {
        prompt: "{{ config_prompt }}".to_string(),
        ..params("triage", &here(), "every day at 9:00")
    };
    let (schedule, _) = schedules::Model::create(&ctx.db, &asked)
        .await
        .unwrap()
        .unwrap();

    // A change keeps what it doesn't name.
    let moved = Changes {
        category: Some(ISSUE.to_string()),
        hron: Some("0 7 * * *".into()),
        ..Changes::default()
    };
    let (changed, category) = schedules::Model::change(&ctx.db, schedule.id, moved)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(category.unwrap().name, ISSUE);
    assert_eq!(
        (
            changed.hron.as_str(),
            changed.command.as_str(),
            changed.prompt.as_str()
        ),
        ("every day at 07:00", "claude -p", "{{ config_prompt }}")
    );
    // And blank takes the category away again.
    let every = Changes {
        category: Some(String::new()),
        ..Changes::default()
    };
    let (changed, category) = schedules::Model::change(&ctx.db, schedule.id, every)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((changed.category_id, category), (None, None));

    // The active configuration's prompt is plain text: whatever it holds, a prompt that
    // places it compiles.
    configure(ctx, "{{ nope }} {% for %}").await;
    let again = Changes {
        command: Some("cat".to_string()),
        ..Changes::default()
    };
    let changed = schedules::Model::change(&ctx.db, schedule.id, again).await;
    assert_eq!(changed.unwrap().unwrap().0.command, "cat");
}

#[tokio::test]
#[serial]
async fn a_schedule_is_due_for_the_whole_minute_of_its_time_on_the_servers_clock() {
    let boot = boot_test::<App>().await.unwrap();
    let mut asked = params("review", &here(), "30  6 * * 1-5");
    asked.command = " claude -p --model opus ".to_string();
    let (schedule, _) = schedules::Model::create(&boot.app_context.db, &asked)
        .await
        .unwrap()
        .unwrap();
    // Kept as hron writes it, so the page shows how m8r read it.
    assert_eq!(schedule.hron, "every weekday at 06:30");
    assert_eq!(schedule.command, "claude -p --model opus");

    // A tick starts some seconds into its minute.
    assert!(schedule.is_due(&chicago(16, 6, 30, 0)));
    assert!(schedule.is_due(&chicago(16, 6, 30, 42)));
    assert!(!schedule.is_due(&chicago(16, 6, 31, 0)));
    assert!(!schedule.is_due(&chicago(16, 18, 30, 0)));
    let saturday = chicago(14, 6, 30, 0);
    assert!(!schedule.is_due(&saturday));
    // The clock is the server's: on a server in UTC, the same moment is 12:30.
    let utc = chicago(16, 6, 30, 0).in_tz("UTC").unwrap();
    assert!(!schedule.is_due(&utc));

    assert_eq!(schedule.next(&saturday), Some(chicago(16, 6, 30, 0)));
}

#[tokio::test]
#[serial]
async fn a_schedule_can_name_a_time_zone_of_its_own() {
    let boot = boot_test::<App>().await.unwrap();
    let asked = params("triage", &here(), "every day at 09:00 in America/New_York");
    let (schedule, _) = schedules::Model::create(&boot.app_context.db, &asked)
        .await
        .unwrap()
        .unwrap();

    // 09:00 in New York is 08:00 in Chicago.
    assert!(schedule.is_due(&chicago(16, 8, 0, 5)));
    assert!(!schedule.is_due(&chicago(16, 9, 0, 0)));
    // The next time reads on the server's clock.
    assert_eq!(
        schedule.next(&chicago(16, 8, 1, 0)),
        Some(chicago(17, 8, 0, 0))
    );
}

#[tokio::test]
#[serial]
async fn a_schedule_runs_at_each_of_its_times() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    // One to a line, as a form sends them: a browser ends a line with a return too. A line can
    // be cron, and one that says the same as another counts once.
    let asked = params(
        "triage",
        &here(),
        "every weekday at 6:00, 15:30\r\n\r\n 0 12 * * 6 \r\nEvery weekday at 06:00, 15:30",
    );
    let (schedule, _) = schedules::Model::create(db, &asked).await.unwrap().unwrap();
    assert_eq!(
        schedule.times().collect::<Vec<_>>(),
        ["every weekday at 06:00, 15:30", "every saturday at 12:00"]
    );
    assert_eq!(
        schedule.hron,
        "every weekday at 06:00, 15:30\nevery saturday at 12:00"
    );

    // 2026-11-16 is a Monday, and the 14th is the Saturday before it.
    assert!(schedule.is_due(&chicago(16, 6, 0, 30)));
    assert!(schedule.is_due(&chicago(16, 15, 30, 0)));
    assert!(!schedule.is_due(&chicago(16, 12, 0, 0)));
    assert!(schedule.is_due(&chicago(14, 12, 0, 0)));
    assert!(!schedule.is_due(&chicago(14, 6, 0, 0)));
    // The next time is the first that any of them has.
    assert_eq!(
        schedule.next(&chicago(14, 6, 0, 0)),
        Some(chicago(14, 12, 0, 0))
    );
    assert_eq!(
        schedule.next(&chicago(14, 12, 0, 0)),
        Some(chicago(16, 6, 0, 0))
    );
    assert_eq!(
        schedule.next(&chicago(16, 6, 0, 0)),
        Some(chicago(16, 15, 30, 0))
    );

    // One line that m8r can't read keeps the schedule from being saved, and hron says which.
    let bad = params("triage", &here(), "every day at 9:00\nevery day at 6am");
    let problem = schedules::Model::create(db, &bad).await.unwrap();
    assert_eq!(
        problem.unwrap_err(),
        "error: unknown keyword 'am'\n  every day at 6am\n                ^^"
    );
}

#[tokio::test]
#[serial]
async fn a_schedule_keeps_each_of_its_repos_once() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let here = here();
    // Two clones, and the first one again, written another way.
    let asked = params(
        "triage",
        &format!(" {here}/src \r\n\r\n{here}/tests\r\n{here}/src/"),
        "every day at 9:00",
    );
    let (schedule, _) = schedules::Model::create(db, &asked).await.unwrap().unwrap();
    assert_eq!(schedule.repos, format!("{here}/src\n{here}/tests"));
    assert_eq!(
        schedule.repos().collect::<Vec<_>>(),
        [format!("{here}/src"), format!("{here}/tests")]
    );

    // Each one must be there.
    let asked = params(
        "triage",
        &format!("{here}/src\n{here}/no-such-clone"),
        "every day at 9:00",
    );
    let problem = schedules::Model::create(db, &asked).await.unwrap();
    assert_eq!(
        problem.unwrap_err(),
        format!(
            "A repo must be a clone in the home of m8r's user, such as ~/git/group/project, and \
             \"{here}/no-such-clone\" isn't a directory there."
        )
    );
}

#[tokio::test]
#[serial]
async fn a_repo_that_leaves_a_schedule_leaves_nothing_behind() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let (src, tests) = (format!("{}/src", here()), format!("{}/tests", here()));
    let asked = params("triage", &format!("{src}\n{tests}"), "every day at 9:00");
    let (schedule, _) = schedules::Model::create(db, &asked).await.unwrap().unwrap();
    let open = || async move {
        let open = errors::Model::open(db).await.unwrap();
        open.into_iter()
            .map(|error| error.subject)
            .collect::<Vec<_>>()
    };
    let fail = |run: runs::Model| async move {
        let run = run.into_active_model();
        run.finished(db, Status::Failed, String::new(), "it broke".to_string())
            .await
            .unwrap()
    };
    let (in_src, in_tests) = (
        format!("schedule {} in {src}", schedule.id),
        format!("schedule {} in {tests}", schedule.id),
    );

    // A run failed in each clone: each has an error of its own.
    for run in runs::Model::queue(db, &schedule).await.unwrap() {
        fail(start(db, run, "p").await).await;
    }
    assert_eq!(open().await, [in_src.as_str(), in_tests.as_str()]);
    // In the second clone, one run is in progress and the next one waits. In the first, one
    // waits.
    let mut queued = runs::Model::queue(db, &schedule).await.unwrap();
    let in_progress = start(db, queued.remove(1), "p").await;
    let queued = runs::Model::queue(db, &schedule).await.unwrap();
    let (waits_in_src, waits_in_tests) = (&queued[0], &queued[1]);
    assert_ne!(waits_in_tests.id, in_progress.id);

    // The schedule drops the second clone.
    let only_src = Changes {
        repos: Some(src.as_str().into()),
        ..Changes::default()
    };
    let (schedule, _) = schedules::Model::change(db, schedule.id, only_src)
        .await
        .unwrap()
        .unwrap();

    // Nothing waits to start there, and its error is over. The first clone's are as they were.
    assert!(runs::Model::find(db, waits_in_tests.id).await.is_err());
    let kept = runs::Model::find(db, waits_in_src.id).await.unwrap();
    assert_eq!(kept.status, "queued");
    assert_eq!(open().await, [in_src.as_str()]);
    // The run that was in progress there fails: no later run could clear an error, so it
    // raises none.
    let failed = fail(in_progress).await;
    assert_eq!(failed.status, "failed");
    assert_eq!(open().await, [in_src.as_str()]);

    // The first clone's next run goes well, and its error is over too.
    let run = start(db, kept, "p").await.into_active_model();
    run.finished(db, Status::Ok, String::new(), String::new())
        .await
        .unwrap();
    assert!(open().await.is_empty());

    // A schedule that's removed takes its errors with it.
    for run in runs::Model::queue(db, &schedule).await.unwrap() {
        fail(start(db, run, "p").await).await;
    }
    assert_eq!(open().await, [in_src.as_str()]);
    schedules::Model::remove(db, schedule.id).await.unwrap();
    assert!(open().await.is_empty());
    // The list keeps them all.
    assert_eq!(errors::Model::list(db, 10).await.unwrap().len(), 3);
}
