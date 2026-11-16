use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::configurations::{self, Changes, Params},
    prompts,
};
use serial_test::serial;

fn params(name: &str, system_message: &str) -> Params {
    Params {
        name: name.to_string(),
        system_message: system_message.to_string(),
    }
}

async fn create(db: &DatabaseConnection, params: &Params) -> configurations::Model {
    let made = configurations::Model::create(db, params).await;
    made.unwrap().unwrap()
}

#[test]
fn a_new_configuration_starts_with_the_default_system_message() {
    let starter = Params::starter();
    assert_eq!(starter.system_message, prompts::DEFAULT_SYSTEM_MESSAGE);
    assert!(starter.name.is_empty());
}

#[tokio::test]
#[serial]
async fn a_configuration_needs_a_name_and_a_system_message_that_compiles() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let problem = |params: Params| async move {
        let made = configurations::Model::create(db, &params).await;
        made.unwrap().unwrap_err()
    };

    assert_eq!(
        problem(params(" ", "x")).await,
        "Give the configuration a name."
    );
    let unknown = problem(params("One", "{{ nope }}")).await;
    assert!(
        unknown.starts_with("The system message doesn't compile."),
        "{unknown}"
    );
    assert!(unknown.contains("nope"), "{unknown}");
    // The system message can't hold itself.
    let itself = problem(params("One", "{{ system_message }}")).await;
    assert!(itself.starts_with("The system message doesn't compile."));
    assert!(configurations::Model::list(db).await.unwrap().is_empty());

    // The default compiles, and so does every variable but that one.
    create(
        db,
        &Params {
            name: "One".to_string(),
            ..Params::starter()
        },
    )
    .await;
    let all = "{{ skill }} {{ watchlist }} {{ category }} {{ artifacts_dir }} \
               {% for item in watchlist_items %}{{ item.url }}{% endfor %}";
    let two = create(db, &params(" Two ", all)).await;
    assert_eq!(two.name, "Two");

    // A change keeps what it doesn't name, and is checked like the rest.
    let renamed = Changes {
        name: Some("Evenings".to_string()),
        ..Changes::default()
    };
    let changed = configurations::Model::change(db, two.id, renamed).await;
    let changed = changed.unwrap().unwrap();
    assert_eq!(
        (changed.name.as_str(), changed.system_message.as_str()),
        ("Evenings", all)
    );
    let broken = Changes {
        system_message: Some("{% for %}".to_string()),
        ..Changes::default()
    };
    let refused = configurations::Model::change(db, two.id, broken).await;
    assert!(refused.unwrap().is_err());
}

#[tokio::test]
#[serial]
async fn at_most_one_configuration_is_active() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let one = create(db, &params("One", "")).await;
    let two = create(db, &params("Two", "")).await;
    assert!(configurations::Model::active(db).await.unwrap().is_none());

    one.clone().into_active_model().activated(db).await.unwrap();
    let two = two.into_active_model().activated(db).await.unwrap();

    let active = configurations::Model::active(db).await.unwrap().unwrap();
    assert_eq!(active.id, two.id);
    assert!(
        !configurations::Model::find(db, one.id)
            .await
            .unwrap()
            .active
    );

    // The database refuses a second one, whatever the code does.
    let mut second = one.into_active_model();
    second.active = Set(true);
    assert!(second.update(db).await.is_err());

    two.into_active_model().deactivated(db).await.unwrap();
    assert!(configurations::Model::active(db).await.unwrap().is_none());
}
