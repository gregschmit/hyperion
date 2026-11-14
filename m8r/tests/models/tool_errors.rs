use loco_rs::testing::prelude::*;
use m8r::{app::App, models::tool_errors};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn an_error_is_one_row_for_a_tool_and_a_host() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    tool_errors::Model::set(db, "glab", "git.example.com", "401 Unauthorized")
        .await
        .unwrap();
    tool_errors::Model::set(db, "gh", "", "It isn't installed.")
        .await
        .unwrap();
    tool_errors::Model::set(db, "glab", "git.example.com", "403 Forbidden")
        .await
        .unwrap();

    let errors = tool_errors::Model::list(db).await.unwrap();
    let rows: Vec<_> = errors
        .iter()
        .map(|error| {
            (
                error.tool.as_str(),
                error.host.as_str(),
                error.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("gh", "", "It isn't installed."),
            ("glab", "git.example.com", "403 Forbidden"),
        ]
    );

    tool_errors::Model::clear(db, "glab", "git.example.com")
        .await
        .unwrap();
    tool_errors::Model::clear(db, "glab", "never.failed")
        .await
        .unwrap();
    assert_eq!(tool_errors::Model::list(db).await.unwrap().len(), 1);
}
