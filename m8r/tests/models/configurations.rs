use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        configurations::{self, Changes, Params},
        errors,
    },
};
use serial_test::serial;

fn params(name: &str, prompt: &str) -> Params {
    Params {
        name: name.to_string(),
        prompt: prompt.to_string(),
    }
}

async fn create(db: &DatabaseConnection, params: &Params) -> configurations::Model {
    let made = configurations::Model::create(db, params).await;
    made.unwrap().unwrap()
}

#[tokio::test]
#[serial]
async fn a_configuration_needs_a_name_and_takes_any_text_for_its_prompt() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;

    let nameless = configurations::Model::create(db, &params(" ", "x")).await;
    assert_eq!(
        nameless.unwrap().unwrap_err(),
        "Give the configuration a name."
    );
    assert!(configurations::Model::list(db).await.unwrap().is_empty());

    // A blank prompt is fine: m8r's own part is in every m8r.
    create(db, &params("One", "")).await;
    // The prompt is plain text, and kept as written: nothing in it is a tag to check.
    let any = "{{ nope }} {% for %} {{ config_prompt }}\n";
    let two = create(db, &params(" Two ", any)).await;
    assert_eq!((two.name.as_str(), two.prompt.as_str()), ("Two", any));

    // A change keeps what it doesn't name, and is checked like the rest.
    let renamed = Changes {
        name: Some("Evenings".to_string()),
        ..Changes::default()
    };
    let changed = configurations::Model::change(db, two.id, renamed).await;
    let changed = changed.unwrap().unwrap();
    assert_eq!(
        (changed.name.as_str(), changed.prompt.as_str()),
        ("Evenings", any)
    );
    let nameless = Changes {
        name: Some(" ".to_string()),
        ..Changes::default()
    };
    let refused = configurations::Model::change(db, two.id, nameless).await;
    assert_eq!(
        refused.unwrap().unwrap_err(),
        "Give the configuration a name."
    );
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

#[tokio::test]
#[serial]
async fn no_active_configuration_is_an_error_until_one_is_active() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    // What a tick and a page ask: is m8r set up?
    let raised = |db| async move {
        errors::Model::check_setup(db).await.unwrap();
        let open = errors::Model::open(db).await.unwrap();
        open.iter()
            .any(|error| error.subject == errors::CONFIGURATION)
    };
    assert!(raised(db).await);

    // Writing one isn't enough: it has to be active.
    let one = create(db, &params("One", "")).await;
    assert!(raised(db).await);
    let one = one.into_active_model().activated(db).await.unwrap();
    assert!(!raised(db).await);

    // Removing the active one brings the error back, and so does deactivating it.
    configurations::Model::remove(db, one.id).await.unwrap();
    assert!(raised(db).await);
    let two = create(db, &params("Two", "")).await;
    let two = two.into_active_model().activated(db).await.unwrap();
    assert!(!raised(db).await);
    two.into_active_model().deactivated(db).await.unwrap();
    assert!(raised(db).await);
    let gone = configurations::Model::remove(db, 99).await;
    assert!(matches!(gone, Err(ModelError::EntityNotFound)));
    // The list keeps the two times that it was put right.
    let cleared = errors::Model::list(db, 10).await.unwrap();
    assert_eq!(
        cleared.iter().filter(|it| it.cleared_at.is_some()).count(),
        2
    );
}
