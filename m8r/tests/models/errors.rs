use chrono::{Duration, Utc};
use loco_rs::prelude::*;
use m8r::{app::App, models::errors};
use serial_test::serial;

async fn open(db: &DatabaseConnection) -> Vec<(String, String)> {
    errors::Model::open(db)
        .await
        .unwrap()
        .into_iter()
        .map(|error| (error.subject, error.message))
        .collect()
}

fn pair(subject: &str, message: &str) -> (String, String) {
    (subject.to_string(), message.to_string())
}

#[tokio::test]
#[serial]
async fn a_subject_has_one_open_error_until_it_is_cleared() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    errors::Model::raise(db, "gh", "It's logged out.", None)
        .await
        .unwrap();
    errors::Model::raise(db, "API", "No answer.", None)
        .await
        .unwrap();
    assert_eq!(
        open(db).await,
        [pair("gh", "It's logged out."), pair("API", "No answer.")]
    );

    // Raised again, it's the same error, with what's wrong now.
    let first = errors::Model::open(db).await.unwrap()[0].clone();
    errors::Model::raise(db, "gh", "It's logged out.", None)
        .await
        .unwrap();
    errors::Model::raise(db, "gh", "It's gone.", None)
        .await
        .unwrap();
    let again = errors::Model::open(db).await.unwrap()[0].clone();
    assert_eq!(
        (again.id, again.created_at, again.message.as_str()),
        (first.id, first.created_at, "It's gone.")
    );
    assert_eq!(open(db).await.len(), 2);

    // Cleared, it's over, and the list keeps it. Nothing else clears with it.
    errors::Model::clear(db, "gh").await.unwrap();
    errors::Model::clear(db, "glab").await.unwrap();
    assert_eq!(open(db).await, [pair("API", "No answer.")]);
    let listed = errors::Model::list(db, 10).await.unwrap();
    let state: Vec<_> = listed
        .iter()
        .map(|error| (error.subject.as_str(), error.cleared_at.is_some()))
        .collect();
    assert_eq!(state, [("API", false), ("gh", true)]);

    // What goes wrong again is a new error.
    errors::Model::raise(db, "gh", "It's logged out again.", None)
        .await
        .unwrap();
    assert_eq!(open(db).await.len(), 2);
    assert_eq!(errors::Model::list(db, 10).await.unwrap().len(), 3);
    // And the list shows no more of the old ones than it's asked for.
    assert_eq!(errors::Model::list(db, 0).await.unwrap().len(), 2);
}

#[tokio::test]
#[serial]
async fn a_check_that_knows_both_raises_or_clears() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    errors::Model::set(db, "API", Some("No answer."))
        .await
        .unwrap();
    assert_eq!(open(db).await, [pair("API", "No answer.")]);
    errors::Model::set(db, "API", None).await.unwrap();
    assert!(open(db).await.is_empty());
    // Clearing what isn't raised changes nothing.
    errors::Model::set(db, "API", None).await.unwrap();
    assert_eq!(errors::Model::list(db, 10).await.unwrap().len(), 1);
}

#[tokio::test]
#[serial]
async fn the_list_forgets_what_was_cleared_long_ago() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    for subject in ["old", "new", "open"] {
        errors::Model::raise(db, subject, "x", None).await.unwrap();
    }
    errors::Model::clear(db, "new").await.unwrap();
    // Cleared more than 30 days ago.
    let old = errors::Model::open(db).await.unwrap()[0].clone();
    let mut old = old.into_active_model();
    old.cleared_at = Set(Some((Utc::now() - Duration::days(31)).into()));
    old.update(db).await.unwrap();

    errors::Model::forget(db).await.unwrap();

    let left: Vec<_> = errors::Model::list(db, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|error| error.subject)
        .collect();
    assert_eq!(left, ["open", "new"]);
}
