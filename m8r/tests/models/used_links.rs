use chrono::{Duration, Utc};
use loco_rs::prelude::*;
use m8r::{app::App, models::used_links};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn a_single_use_link_is_spent_once() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let later = Utc::now() + Duration::hours(1);

    assert!(used_links::Model::spend(db, "abc", later).await.unwrap());
    assert!(!used_links::Model::spend(db, "abc", later).await.unwrap());
    assert!(used_links::Model::spend(db, "def", later).await.unwrap());
}

#[tokio::test]
#[serial]
async fn a_row_goes_when_its_link_would_have_expired_anyway() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let earlier = Utc::now() - Duration::minutes(1);
    assert!(used_links::Model::spend(db, "old", earlier).await.unwrap());

    assert!(
        used_links::Model::spend(db, "new", Utc::now() + Duration::hours(1))
            .await
            .unwrap()
    );

    let left: Vec<String> = used_links::Entity::find()
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|link| link.token_id)
        .collect();
    assert_eq!(left, ["new"]);
}
