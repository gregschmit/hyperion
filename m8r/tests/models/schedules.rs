use jiff::{civil::date, Zoned};
use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::schedules::{self, parse, Changes, Operation, Params},
    prompts,
};
use serial_test::serial;

use crate::support::{configure, watch, ISSUE, PR};

fn params(operation: &str, repo: &str, hron: &str) -> Params {
    Params {
        operation: operation.to_string(),
        repo: repo.to_string(),
        hron: hron.to_string(),
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
    assert!(saved(params("triage", "relative/path", "0 6 * * 1-5"))
        .await
        .is_err());
    assert!(saved(params("triage", &here(), "at six")).await.is_err());
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
async fn a_schedules_prompt_is_checked_against_the_active_system_message() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    watch(ctx, "https://github.com/o/r/issues/1", ISSUE).await;
    let asked = Params {
        prompt: "{{ system_message }}".to_string(),
        ..params("triage", &here(), "every day at 9:00")
    };
    let (schedule, _) = schedules::Model::create(&ctx.db, &asked)
        .await
        .unwrap()
        .unwrap();

    // A change keeps what it doesn't name.
    let moved = Changes {
        category: Some(ISSUE.to_string()),
        hron: Some("0 7 * * *".to_string()),
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
        ("every day at 07:00", "claude -p", "{{ system_message }}")
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

    // The system message that's active now doesn't compile, so no prompt with it does.
    let mut broken = configure(ctx, "").await.into_active_model();
    broken.system_message = Set("{{ nope }}".to_string());
    broken.update(&ctx.db).await.unwrap();
    let again = Changes {
        command: Some("cat".to_string()),
        ..Changes::default()
    };
    let refused = schedules::Model::change(&ctx.db, schedule.id, again).await;
    let problem = refused.unwrap().unwrap_err();
    assert!(
        problem.starts_with("The system message doesn't compile."),
        "{problem}"
    );
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
