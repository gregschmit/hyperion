use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::categories::{self, Params},
};
use serial_test::serial;

use crate::support::{ISSUE, PR};

fn named(name: &str) -> Params {
    Params {
        name: name.to_string(),
    }
}

#[tokio::test]
#[serial]
async fn every_m8r_starts_with_two_categories() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let names: Vec<_> = categories::Model::list(db)
        .await
        .unwrap()
        .into_iter()
        .map(|category| category.name)
        .collect();
    assert_eq!(names, [ISSUE, PR]);
}

#[tokio::test]
#[serial]
async fn a_category_is_one_name_of_255_characters_at_most() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    let epic = categories::Model::create(db, &named(" Epic ")).await;
    let epic = epic.unwrap().unwrap();
    assert_eq!(epic.name, "Epic");
    // Characters, not bytes.
    let long = "é".repeat(255);
    assert!(categories::Model::create(db, &named(&long))
        .await
        .unwrap()
        .is_ok());
    let longer = categories::Model::create(db, &named(&"é".repeat(256))).await;
    assert_eq!(
        longer.unwrap().unwrap_err(),
        "A category's name can have 255 characters."
    );
    let blank = categories::Model::create(db, &named("  ")).await;
    assert_eq!(blank.unwrap().unwrap_err(), "Give the category a name.");
    let twice = categories::Model::create(db, &named("Epic")).await;
    assert_eq!(
        twice.unwrap().unwrap_err(),
        "There's already a category \"Epic\"."
    );

    // The same goes for a new name, but a category can keep its own.
    let taken = epic.clone().renamed(db, &named(ISSUE)).await;
    assert!(taken.unwrap().is_err());
    assert!(epic
        .clone()
        .renamed(db, &named("Epic"))
        .await
        .unwrap()
        .is_ok());
    let release = epic.renamed(db, &named("Release")).await.unwrap().unwrap();
    assert_eq!(release.name, "Release");
    assert!(categories::Model::named(db, "Epic").await.unwrap().is_err());
    assert_eq!(
        categories::Model::named(db, " Release ")
            .await
            .unwrap()
            .unwrap()
            .id,
        release.id
    );

    assert!(release.removed(db).await.unwrap().is_ok());
    assert_eq!(categories::Model::list(db).await.unwrap().len(), 3);
}
