use loco_rs::prelude::*;
use m8r::{
    app::App,
    models::{
        configurations::{self, Operation, Params, DEFAULT_SYSTEM_MESSAGE},
        items,
    },
};
use serial_test::serial;

fn params(name: &str, triage_prompt: &str) -> Params {
    Params {
        name: name.to_string(),
        triage_prompt: triage_prompt.to_string(),
        ..Params::starter()
    }
}

#[tokio::test]
#[serial]
async fn compiles_the_system_message_into_the_prompt_at_its_spot() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    for (url, priority, note) in [
        ("https://github.com/o/r/issues/1", "medium", ""),
        (
            "https://git.example.com/g/p/-/merge_requests/2",
            "high",
            "Ship by <Friday>",
        ),
    ] {
        let item = items::Params {
            url: url.to_string(),
            priority: priority.to_string(),
            note: note.to_string(),
        };
        items::Model::create(db, &item).await.unwrap();
    }
    let items = items::Model::list(db).await.unwrap();
    let configuration =
        configurations::Model::create(db, &params("One", "Before.\n{{ system_message }}\nAfter."))
            .await
            .unwrap();

    let prompt = configuration
        .compile(Operation::Triage, &items, "/data/artifacts/9")
        .unwrap();

    assert!(prompt.starts_with("Before.\nYou are running in m8r"));
    assert!(prompt.ends_with("\nAfter."));
    // The urgent one first, and nothing escaped: a prompt isn't HTML.
    let high =
        prompt.find("- high: https://git.example.com/g/p/-/merge_requests/2 Ship by <Friday>");
    let medium = prompt.find("- medium: https://github.com/o/r/issues/1");
    assert!(high.is_some() && high < medium, "{prompt}");
    assert!(prompt.contains("make a directory in\n/data/artifacts/9, named"));
}

#[test]
fn a_prompt_without_the_spot_gets_no_system_message() {
    let prompt = params("One", "Just triage.")
        .compile(Operation::Triage, &[], "/x")
        .unwrap();
    assert_eq!(prompt, "Just triage.");
}

#[test]
fn the_default_system_message_says_when_nothing_has_priority() {
    let prompt = params("One", "{{ system_message }}")
        .compile(Operation::Triage, &[], "/x")
        .unwrap();
    assert!(prompt.contains("until it closes:\n(none)\n"), "{prompt}");
}

#[test]
fn a_template_that_does_not_compile_is_a_problem() {
    assert!(params("One", "{{ system_message }}").problem(&[]).is_none());

    let unknown = params("One", "{{ nope }}").problem(&[]).unwrap();
    assert!(
        unknown.starts_with("The triage prompt doesn't compile."),
        "{unknown}"
    );
    assert!(unknown.contains("nope"), "{unknown}");

    let mut broken = params("One", "{{ system_message }}");
    broken.system_message = "{% for %}".to_string();
    let problem = broken.problem(&[]).unwrap();
    assert!(
        problem.starts_with("The system message doesn't compile."),
        "{problem}"
    );

    assert_eq!(
        params(" ", "x").problem(&[]).unwrap(),
        "Give the configuration a name."
    );
}

#[test]
fn a_new_configuration_starts_with_the_default_system_message_and_no_prompts() {
    let starter = Params::starter();
    assert_eq!(starter.system_message, DEFAULT_SYSTEM_MESSAGE);
    assert!(starter.triage_prompt.is_empty());
    assert_eq!(Operation::parse("review"), Some(Operation::Review));
    assert_eq!(Operation::parse("deploy"), None);
}

#[tokio::test]
#[serial]
async fn at_most_one_configuration_is_active() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let one = configurations::Model::create(db, &params("One", ""))
        .await
        .unwrap();
    let two = configurations::Model::create(db, &params("Two", ""))
        .await
        .unwrap();
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
