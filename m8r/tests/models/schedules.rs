use chrono::{Local, TimeZone};
use loco_rs::testing::prelude::*;
use m8r::{
    app::App,
    models::schedules::{self, parse_cron, Params},
};
use serial_test::serial;

fn params(operation: &str, repo: &str, cron: &str) -> Params {
    Params {
        operation: operation.to_string(),
        repo: repo.to_string(),
        cron: cron.to_string(),
    }
}

fn here() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_cron_expression_has_five_fields() {
    assert!(parse_cron("0 6 * * 1-5").is_ok());
    assert!(parse_cron("  */15 8-18 * * *  ").is_ok());
    // Not Loco's own form, with seconds first.
    assert!(parse_cron("0 0 6 * * 1-5").is_err());
    assert!(parse_cron("0 6 * *").is_err());
    assert!(parse_cron("61 6 * * *").is_err());
    assert!(parse_cron("").is_err());
}

#[test]
fn a_schedule_needs_an_operation_a_clone_and_a_time() {
    assert!(params("triage", &here(), "0 6 * * 1-5").problem().is_none());
    assert!(params("deploy", &here(), "0 6 * * 1-5").problem().is_some());
    assert!(params("triage", "/no/such/clone", "0 6 * * 1-5")
        .problem()
        .is_some());
    assert!(params("triage", "relative/path", "0 6 * * 1-5")
        .problem()
        .is_some());
    assert!(params("triage", &here(), "at six").problem().is_some());
}

#[tokio::test]
#[serial]
async fn a_schedule_is_due_for_the_whole_minute_of_its_time() {
    let boot = boot_test::<App>().await.unwrap();
    let schedule = schedules::Model::create(
        &boot.app_context.db,
        &params("review", &here(), "30  6 * * 1-5"),
    )
    .await
    .unwrap();
    assert_eq!(schedule.cron, "30 6 * * 1-5");

    // 2026-11-16 is a Monday. A tick starts some seconds into its minute.
    let monday = |hour, minute, second| {
        Local
            .with_ymd_and_hms(2026, 11, 16, hour, minute, second)
            .unwrap()
    };
    assert!(schedule.is_due(&monday(6, 30, 0)));
    assert!(schedule.is_due(&monday(6, 30, 42)));
    assert!(!schedule.is_due(&monday(6, 31, 0)));
    assert!(!schedule.is_due(&monday(18, 30, 0)));
    let saturday = Local.with_ymd_and_hms(2026, 11, 14, 6, 30, 0).unwrap();
    assert!(!schedule.is_due(&saturday));

    assert_eq!(schedule.next(&saturday), Some(monday(6, 30, 0)));
}
