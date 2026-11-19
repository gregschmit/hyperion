use std::path::Path;

use jiff::{civil::date, Zoned};
use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        errors,
        runs::{self, Status},
        schedules::{self, parse, Changes, Items, Params},
        watchlist_items::Category,
    },
    prompts,
};
use serial_test::serial;

use crate::support::{self, queue_all, start, watch, ISSUES, REQUESTS};

fn params(name: &str, repo: &str, hron: &str) -> Params {
    Params {
        name: name.to_string(),
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

/// The clone that the tests run in, as one of m8r's repos, and its path.
async fn repo_here(db: &DatabaseConnection) -> String {
    support::repo(db, &here(), "").await.path
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

#[tokio::test]
#[serial]
async fn a_schedule_names_a_repo_by_its_clones_path_however_it_is_written() {
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
    let repo = support::repo(db, &format!("~/{}", inside.display()), "").await;

    // The schedule names it by its full path, and it's that repo.
    let made = schedules::Model::create(db, &params("triage", &here(), "every day at 9:00")).await;
    let (schedule, repos) = made.unwrap().unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].id, repo.id);
    assert_eq!(schedule.repos(db).await.unwrap()[0].path, repo.path);
}

#[test]
fn a_new_schedule_starts_with_the_agents_command_and_m8rs_parts_in_its_prompt() {
    let starter = Params::starter("claude -p --model opus");
    assert_eq!(starter.name, "");
    assert_eq!(starter.command, "claude -p --model opus");
    assert_eq!(starter.prompt, prompts::STARTER);
    // Its prompt names the watchlist, so it starts with every item, and with no cutoff.
    assert_eq!((starter.category.as_str(), starter.max_items), ("all", 0));
}

#[test]
fn a_schedule_takes_all_items_a_categorys_or_none() {
    assert_eq!(
        Items::CHOICES.map(Items::as_str),
        ["", "all", "issues", "pull/merge requests"]
    );
    assert_eq!(Items::parse(" "), Some(Items::None));
    assert_eq!(Items::parse("all"), Some(Items::All));
    assert_eq!(
        Items::parse("pull/merge requests"),
        Some(Items::Of(Category::Requests))
    );
    assert_eq!(Items::parse("Issue"), None);
}

#[tokio::test]
#[serial]
async fn a_schedule_needs_a_name_a_repo_a_time_and_a_command() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let here = repo_here(db).await;
    let saved = |params: Params| async move {
        let made = schedules::Model::create(db, &params).await;
        made.unwrap().map(|(schedule, _)| schedule)
    };

    assert!(saved(params("triage", &here, "0 6 * * 1-5")).await.is_ok());
    assert!(saved(params("nightly docs", &here, "every 12 hours"))
        .await
        .is_ok());

    // A name is what you call it, and it's the only one of that name.
    assert_eq!(
        saved(params(" ", &here, "0 6 * * 1-5")).await.unwrap_err(),
        "Give the schedule a name."
    );
    assert_eq!(
        saved(params(" triage ", &here, "0 6 * * 1-5"))
            .await
            .unwrap_err(),
        "There's already a schedule \"triage\"."
    );

    // A repo is one that m8r has.
    assert_eq!(
        saved(params("deploy", " ", "0 6 * * 1-5"))
            .await
            .unwrap_err(),
        "Say which repos the schedule runs in."
    );
    assert_eq!(
        saved(params("deploy", "/no/such/clone", "0 6 * * 1-5"))
            .await
            .unwrap_err(),
        "\"/no/such/clone\" isn't one of m8r's repos. Add it to the repos first."
    );
    // A clone that's there isn't a repo until you add it.
    let unknown = format!("{here}/src");
    assert!(saved(params("deploy", &unknown, "0 6 * * 1-5"))
        .await
        .unwrap_err()
        .ends_with("isn't one of m8r's repos. Add it to the repos first."));

    assert!(saved(params("deploy", &here, "at six")).await.is_err());
    assert_eq!(
        saved(params("deploy", &here, " \n")).await.unwrap_err(),
        "Give the schedule an hron: when it runs, such as every weekday at 06:00."
    );
    let no_command = Params {
        command: "  ".to_string(),
        ..params("deploy", &here, "0 6 * * 1-5")
    };
    assert_eq!(
        saved(no_command).await.unwrap_err(),
        "Say which command the schedule runs."
    );

    // It takes every item of its repo, a category's, or none: then its runs get no items.
    // And it can take only the first ones.
    let review = |name: &str, category: &str, max_items: u32| Params {
        category: category.to_string(),
        max_items,
        ..params(name, &here, "every day at 9:00")
    };
    let schedule = saved(review("review", REQUESTS, 50)).await.unwrap();
    assert_eq!(schedule.items(), Items::Of(Category::Requests));
    assert_eq!(
        (schedule.category.as_str(), schedule.max_items()),
        (REQUESTS, 50)
    );
    let schedule = saved(review("everything", " all ", 0)).await.unwrap();
    assert_eq!((schedule.items(), schedule.max_items()), (Items::All, 0));
    let schedule = saved(review("nothing", " ", 0)).await.unwrap();
    assert_eq!(schedule.items(), Items::None);
    assert!(saved(review("epics", "Epic", 0))
        .await
        .unwrap_err()
        .starts_with("A schedule's category is all, issues or pull/merge requests"));

    // Its prompt compiles with what a run would get.
    let prompted = |name: &str, prompt: &str| Params {
        prompt: prompt.to_string(),
        ..params(name, &here, "every day at 9:00")
    };
    assert!(saved(prompted("starter", prompts::STARTER)).await.is_ok());
    assert!(saved(prompted("blank", "")).await.is_ok());
    let broken = saved(prompted("broken", "{{ items }}")).await.unwrap_err();
    assert!(
        broken.starts_with("The prompt doesn't compile."),
        "{broken}"
    );
    let names: Vec<String> = schedules::Model::list(db)
        .await
        .unwrap()
        .into_iter()
        .map(|(schedule, _)| schedule.name)
        .collect();
    // By name.
    assert_eq!(
        names,
        [
            "blank",
            "everything",
            "nightly docs",
            "nothing",
            "review",
            "starter",
            "triage"
        ]
    );
}

#[tokio::test]
#[serial]
async fn a_change_to_a_schedule_keeps_what_it_does_not_name() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    let here = repo_here(&ctx.db).await;
    watch(ctx, "https://github.com/o/r/issues/1").await;
    let asked = Params {
        prompt: "{{ watchlist }}".to_string(),
        max_items: 5,
        ..params("triage", &here, "every day at 9:00")
    };
    let (schedule, _) = schedules::Model::create(&ctx.db, &asked)
        .await
        .unwrap()
        .unwrap();

    // A change keeps what it doesn't name.
    let moved = Changes {
        category: Some(ISSUES.to_string()),
        hron: Some("0 7 * * *".into()),
        ..Changes::default()
    };
    let (changed, repos) = schedules::Model::change(&ctx.db, schedule.id, moved)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.category, ISSUES);
    assert_eq!(
        (
            changed.name.as_str(),
            changed.hron.as_str(),
            changed.command.as_str(),
            changed.prompt.as_str(),
            changed.max_items(),
        ),
        (
            "triage",
            "every day at 07:00",
            "claude -p",
            "{{ watchlist }}",
            5
        )
    );
    assert_eq!(repos[0].path, here);
    // And blank takes the items away again. A new name is fine, and its own name is too.
    let every = Changes {
        name: Some("mornings".to_string()),
        category: Some(String::new()),
        max_items: Some(0),
        ..Changes::default()
    };
    let (changed, _) = schedules::Model::change(&ctx.db, schedule.id, every)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (changed.name.as_str(), changed.items(), changed.max_items()),
        ("mornings", Items::None, 0)
    );
}

#[tokio::test]
#[serial]
async fn a_schedule_is_due_for_the_whole_minute_of_its_time_on_the_servers_clock() {
    let boot = boot_test::<App>().await.unwrap();
    let here = repo_here(&boot.app_context.db).await;
    let mut asked = params("review", &here, "30  6 * * 1-5");
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
async fn a_schedule_that_is_not_enabled_starts_nothing_at_its_times() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let (src, tests) = (format!("{}/src", here()), format!("{}/tests", here()));
    for path in [&src, &tests] {
        support::repo(db, path, "").await;
    }
    let asked = params("triage", &format!("{src}\n{tests}"), "30 6 * * 1-5");
    let (schedule, _) = schedules::Model::create(db, &asked).await.unwrap().unwrap();
    let monday = chicago(16, 6, 30, 0);
    let status = |run: &runs::Model| {
        let id = run.id;
        async move { runs::Model::find(db, id).await.ok().map(|run| run.status) }
    };
    // A new schedule is enabled.
    assert!(schedule.enabled && schedule.is_due(&monday));
    assert!(schedule.next(&monday).is_some());

    // In one clone its run is in progress, and in the other one waits.
    let mut queued = queue_all(db, &schedule).await;
    let in_progress = start(db, queued.remove(0), "p").await;
    let waiting = queued.remove(0);

    // Off: its time comes and goes. The run that waited is dropped, and the one in progress
    // goes on.
    let (off, repos) = schedules::Model::switch(db, schedule.id, false)
        .await
        .unwrap();
    assert!(!off.enabled && !off.is_due(&monday));
    assert_eq!(off.next(&monday), None);
    assert_eq!(repos.len(), 2);
    assert_eq!(status(&waiting).await, None);
    assert_eq!(status(&in_progress).await.as_deref(), Some("running"));

    // "Run now" is still yours to press. Its runs wait, also when the schedule is saved again
    // as it is, which is off.
    let asked_for = queue_all(db, &off).await;
    let renamed = Changes {
        name: Some("mornings".to_string()),
        ..Changes::default()
    };
    let (still_off, _) = schedules::Model::change(db, off.id, renamed)
        .await
        .unwrap()
        .unwrap();
    assert!(!still_off.enabled);
    for run in &asked_for {
        assert_eq!(status(run).await.as_deref(), Some("queued"));
    }

    // On again, and off with another change: that's checked like any change, and drops the
    // runs that wait too.
    let on = Changes {
        enabled: Some(true),
        ..Changes::default()
    };
    let (on, _) = schedules::Model::change(db, off.id, on)
        .await
        .unwrap()
        .unwrap();
    assert!(on.is_due(&monday));
    let off_with_cat = Changes {
        enabled: Some(false),
        command: Some("cat".to_string()),
        ..Changes::default()
    };
    let (off, _) = schedules::Model::change(db, on.id, off_with_cat)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((off.enabled, off.command.as_str()), (false, "cat"));
    for run in &asked_for {
        assert_eq!(status(run).await, None);
    }

    // A schedule that can't be saved as it is can still be turned off, and on: here, its
    // prompt doesn't compile.
    let mut broken = off.into_active_model();
    broken.prompt = Set("{{ gone }}".to_string());
    let broken = broken.update(db).await.unwrap();
    let renamed = Changes {
        name: Some("evenings".to_string()),
        ..Changes::default()
    };
    let refused = schedules::Model::change(db, broken.id, renamed).await;
    assert!(refused
        .unwrap()
        .unwrap_err()
        .starts_with("The prompt doesn't compile."));
    for enabled in [true, false] {
        let switch = Changes {
            enabled: Some(enabled),
            ..Changes::default()
        };
        let (switched, _) = schedules::Model::change(db, broken.id, switch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(switched.enabled, enabled);
    }
}

#[tokio::test]
#[serial]
async fn a_schedule_can_name_a_time_zone_of_its_own() {
    let boot = boot_test::<App>().await.unwrap();
    let here = repo_here(&boot.app_context.db).await;
    let asked = params("triage", &here, "every day at 09:00 in America/New_York");
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
    let here = repo_here(db).await;
    // One to a line, as a form sends them: a browser ends a line with a return too. A line can
    // be cron, and one that says the same as another counts once.
    let asked = params(
        "triage",
        &here,
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
    let bad = params("evenings", &here, "every day at 9:00\nevery day at 6am");
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
    let (src, tests) = (format!("{here}/src"), format!("{here}/tests"));
    for path in [&tests, &src] {
        support::repo(db, path, "").await;
    }
    // Two repos, and the first one again, written another way.
    let asked = params(
        "triage",
        &format!(" {src} \r\n\r\n{tests}\r\n{src}/"),
        "every day at 9:00",
    );
    let (schedule, repos) = schedules::Model::create(db, &asked).await.unwrap().unwrap();
    let paths = |repos: Vec<m8r::models::repos::Model>| -> Vec<String> {
        repos.into_iter().map(|repo| repo.path).collect()
    };
    assert_eq!(paths(repos), [src.as_str(), tests.as_str()]);
    assert_eq!(
        paths(schedule.repos(db).await.unwrap()),
        [src.as_str(), tests.as_str()]
    );
    // The list has them too.
    let listed = schedules::Model::list(db).await.unwrap();
    assert_eq!(paths(listed[0].1.clone()), [src.as_str(), tests.as_str()]);

    // Each one must be a repo of m8r's.
    let asked = params(
        "review",
        &format!("{src}\n{here}/migration"),
        "every day at 9:00",
    );
    let problem = schedules::Model::create(db, &asked).await.unwrap();
    assert_eq!(
        problem.unwrap_err(),
        format!("\"{here}/migration\" isn't one of m8r's repos. Add it to the repos first.")
    );
}

#[tokio::test]
#[serial]
async fn a_repo_that_leaves_a_schedule_leaves_nothing_behind() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let (src, tests) = (format!("{}/src", here()), format!("{}/tests", here()));
    for path in [&src, &tests] {
        support::repo(db, path, "").await;
    }
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
    for run in queue_all(db, &schedule).await {
        fail(start(db, run, "p").await).await;
    }
    assert_eq!(open().await, [in_src.as_str(), in_tests.as_str()]);
    // In the second clone, one run is in progress and the next one waits. In the first, one
    // waits.
    let mut queued = queue_all(db, &schedule).await;
    let in_progress = start(db, queued.remove(1), "p").await;
    let queued = queue_all(db, &schedule).await;
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
    for run in queue_all(db, &schedule).await {
        fail(start(db, run, "p").await).await;
    }
    assert_eq!(open().await, [in_src.as_str()]);
    schedules::Model::remove(db, schedule.id).await.unwrap();
    assert!(open().await.is_empty());
    // The list keeps them all.
    assert_eq!(errors::Model::list(db, 10).await.unwrap().len(), 3);
}
