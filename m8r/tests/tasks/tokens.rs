use loco_rs::{app::AppContext, boot::run_task, task, testing::prelude::*};
use m8r::{app::App, models::tokens};
use serial_test::serial;

async fn task(ctx: &AppContext, name: &str, args: &[(&str, &str)]) -> loco_rs::Result<()> {
    let vars = task::Vars::from_cli_args(
        args.iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
    );
    run_task::<App>(ctx, Some(&name.to_string()), &vars).await
}

#[tokio::test]
#[serial]
async fn tokens_are_made_listed_and_revoked_on_the_command_line() {
    let boot = boot_test::<App>().await.unwrap();
    let ctx = &boot.app_context;
    assert!(task(ctx, "tokens", &[]).await.is_ok());

    let duet = [("name", "duet"), ("scope", "watchlist_items")];
    assert!(task(ctx, "token", &duet).await.is_ok());
    let made = tokens::Model::list(&ctx.db).await.unwrap();
    assert_eq!(made.len(), 1);
    assert_eq!(
        (made[0].name.as_str(), made[0].scope.as_str()),
        ("duet", "watchlist_items")
    );
    assert!(task(ctx, "tokens", &[]).await.is_ok());

    // A token says what will use it and what it covers, and a name is there once.
    assert!(task(ctx, "token", &[("name", "mac")]).await.is_err());
    assert!(task(ctx, "token", &[("scope", "all")]).await.is_err());
    assert!(
        task(ctx, "token", &[("name", "mac"), ("scope", "everything")])
            .await
            .is_err()
    );
    assert!(task(ctx, "token", &duet).await.is_err());
    assert_eq!(tokens::Model::list(&ctx.db).await.unwrap().len(), 1);

    assert!(task(ctx, "revoke", &[]).await.is_err());
    assert!(task(ctx, "revoke", &[("name", "mac")]).await.is_err());
    assert!(task(ctx, "revoke", &[("name", "duet")]).await.is_ok());
    assert!(tokens::Model::list(&ctx.db).await.unwrap().is_empty());
}
