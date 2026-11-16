use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::tokens::{self, covers},
};
use serial_test::serial;

#[test]
fn a_scope_covers_its_resources_and_all_covers_every_one() {
    assert!(covers("watchlist_items", "watchlist_items"));
    assert!(!covers("watchlist_items", "schedules"));
    assert!(covers("watchlist_items,runs", "runs"));
    // A name inside another name isn't that name.
    assert!(!covers("watchlist_items", "items"));
    assert!(!covers("watchlist_items", ""));
    for resource in tokens::RESOURCES {
        assert!(covers("all", resource), "{resource}");
    }
}

#[tokio::test]
#[serial]
async fn a_token_is_shown_once_and_kept_as_its_digest() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    let token = tokens::Model::create(db, " duet ", "watchlist_items").await;
    let token = token.unwrap().unwrap();
    assert!(token.starts_with("m8r_"), "{token}");
    assert_eq!(token.len(), 44);

    let row = tokens::Model::of(db, &token).await.unwrap().unwrap();
    assert_eq!(
        (row.name.as_str(), row.scope.as_str()),
        ("duet", "watchlist_items")
    );
    assert_eq!(row.digest.len(), 64);
    assert!(!row.digest.contains(&token[4..]));
    // Another token isn't it, however near.
    assert!(tokens::Model::of(db, &token[..43]).await.unwrap().is_none());
    assert!(tokens::Model::of(db, "").await.unwrap().is_none());

    let another = tokens::Model::create(db, "mac", "all")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(another, token);
    assert_eq!(tokens::Model::list(db).await.unwrap().len(), 2);
}

#[tokio::test]
#[serial]
async fn a_token_needs_a_name_of_its_own_and_a_scope_that_m8r_knows() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let problem = |name: &'static str, scope: &'static str| async move {
        let made = tokens::Model::create(db, name, scope).await;
        made.unwrap().unwrap_err()
    };

    assert_eq!(
        problem(" ", "all").await,
        "Give the token a name, for what will use it."
    );
    for scope in ["", "everything", "watchlist_items,nope", ","] {
        let refused = problem("duet", scope).await;
        assert!(
            refused.starts_with("A scope is all, or one or more of"),
            "{scope}: {refused}"
        );
    }

    // A scope is kept tidy, and all is all.
    tokens::Model::create(db, "duet", " runs , watchlist_items ")
        .await
        .unwrap()
        .unwrap();
    tokens::Model::create(db, "mac", "runs,all")
        .await
        .unwrap()
        .unwrap();
    let scopes: Vec<_> = tokens::Model::list(db)
        .await
        .unwrap()
        .into_iter()
        .map(|token| (token.name, token.scope))
        .collect();
    assert_eq!(
        scopes,
        [
            ("duet".to_string(), "runs,watchlist_items".to_string()),
            ("mac".to_string(), "all".to_string()),
        ]
    );
    assert_eq!(
        problem("duet", "all").await,
        "There's already a token called duet. Revoke it first."
    );

    assert!(tokens::Model::revoke(db, "duet").await.unwrap());
    assert!(!tokens::Model::revoke(db, "duet").await.unwrap());
    assert_eq!(tokens::Model::list(db).await.unwrap().len(), 1);
}
