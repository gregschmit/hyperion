use loco_rs::{boot::run_task, task, testing::prelude::*};
use m8r::app::App;
use serial_test::serial;

async fn link(args: &[(&str, &str)]) -> loco_rs::Result<()> {
    let boot = boot_test::<App>().await.unwrap();
    let vars = task::Vars::from_cli_args(
        args.iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
    );
    run_task::<App>(&boot.app_context, Some(&"link".to_string()), &vars).await
}

#[tokio::test]
#[serial]
async fn prints_a_link_with_or_without_arguments() {
    assert!(link(&[]).await.is_ok());
    assert!(link(&[("hours", "72"), ("once", "true")]).await.is_ok());
}

#[tokio::test]
#[serial]
async fn refuses_arguments_it_cannot_read() {
    assert!(link(&[("hours", "soon")]).await.is_err());
    assert!(link(&[("hours", "0")]).await.is_err());
    assert!(link(&[("once", "maybe")]).await.is_err());
}
