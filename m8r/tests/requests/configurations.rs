use m8r::models::configurations;
use serial_test::serial;

use crate::support::{location, signed_in};

fn form<'a>(name: &'a str, system_message: &'a str, triage: &'a str) -> [(&'a str, &'a str); 5] {
    [
        ("name", name),
        ("system_message", system_message),
        ("triage_prompt", triage),
        ("review_prompt", ""),
        ("develop_prompt", ""),
    ]
}

#[tokio::test]
#[serial]
async fn a_configuration_is_written_seen_compiled_and_made_active() {
    signed_in(|request, ctx| async move {
        assert!(request.get("/configurations").await.text().contains("No configurations"));
        let page = request.get("/configurations/new").await.text();
        assert!(page.contains("You are running in m8r"), "{page}");

        // A template that doesn't compile isn't saved, and Tera says why.
        let res = request
            .post("/configurations")
            .form(&form("Mornings", "Hello {{ nobody }}", "{{ system_message }}"))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("The system message doesn&#39;t compile."), "{page}");
        assert!(page.contains("nobody"), "{page}");
        assert!(configurations::Model::list(&ctx.db).await.unwrap().is_empty());

        let res = request
            .post("/configurations")
            .form(&form(
                "Mornings",
                "You are in m8r. Report to {{ artifacts_dir }}.",
                "{{ system_message }}\nTriage the <new> issues.",
            ))
            .await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/configurations/1");

        // Its page shows each prompt compiled.
        let page = request.get("/configurations/1").await.text();
        assert!(
            page.contains(
                "You are in m8r. Report to /var/lib/m8r/artifacts/1.\nTriage the &lt;new&gt; issues."
            ),
            "{page}"
        );
        assert!(page.contains("Blank: a review run fails."), "{page}");

        let res = request
            .post("/configurations/1")
            .form(&form("Evenings", "x", "{{ system_message }} y"))
            .await;
        assert_eq!(location(&res), "/configurations/1");
        let configuration = configurations::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(configuration.name, "Evenings");
        assert!(!configuration.active);

        assert_eq!(request.post("/configurations/1/activate").await.status_code(), 303);
        assert!(configurations::Model::active(&ctx.db).await.unwrap().is_some());
        assert!(request.get("/configurations").await.text().contains("Deactivate"));
        assert_eq!(request.post("/configurations/1/deactivate").await.status_code(), 303);
        assert!(configurations::Model::active(&ctx.db).await.unwrap().is_none());

        assert_eq!(request.post("/configurations/1/remove").await.status_code(), 303);
        assert_eq!(request.get("/configurations/1").await.status_code(), 404);
    })
    .await;
}
