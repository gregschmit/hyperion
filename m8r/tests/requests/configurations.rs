use m8r::models::configurations;
use serial_test::serial;

use crate::support::{location, signed_in, watch, ISSUE};

fn form<'a>(name: &'a str, prompt: &'a str) -> [(&'a str, &'a str); 2] {
    [("name", name), ("prompt", prompt)]
}

#[tokio::test]
#[serial]
async fn a_configuration_is_written_seen_compiled_and_made_active() {
    signed_in(|request, ctx| async move {
        assert!(request
            .get("/configurations")
            .await
            .text()
            .contains("No configurations"));
        // A new one's prompt is blank: m8r's own part isn't the configuration's to hold.
        let page = request.get("/configurations/new").await.text();
        assert!(page.contains(r#"<textarea id="prompt" name="prompt" rows="12"></textarea>"#));
        assert!(!page.contains("You are running in m8r"), "{page}");
        // The page says what the prompt can use: all but itself.
        assert!(page.contains("<code>{{ m8r_prompt }}</code>"), "{page}");
        assert!(page.contains("<code>{{ watchlist }}</code>"), "{page}");
        assert!(
            !page.contains("<code>{{ config_prompt }}</code></td>"),
            "{page}"
        );

        // A template that doesn't compile isn't saved, and Tera says why.
        let res = request
            .post("/configurations")
            .form(&form("Mornings", "Hello {{ nobody }}"))
            .await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(
            page.contains("The configuration&#39;s prompt doesn&#39;t compile."),
            "{page}"
        );
        assert!(page.contains("nobody"), "{page}");
        assert!(configurations::Model::list(&ctx.db)
            .await
            .unwrap()
            .is_empty());
        // And a configuration has a name.
        let res = request.post("/configurations").form(&form(" ", "x")).await;
        assert!(res.text().contains("Give the configuration a name."));

        watch(&ctx, "https://github.com/o/r/issues/1", ISSUE).await;
        let res = request
            .post("/configurations")
            .form(&form(
                "Mornings",
                "Write in plain words. Report to {{ artifacts_dir }}.\nFollow <these>:\n\
                 {{ watchlist }}",
            ))
            .await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/configurations/1");

        // Its page shows the prompt compiled. The items are a schedule's to choose, so here
        // there are none.
        let page = request.get("/configurations/1").await.text();
        assert!(
            page.contains(
                "<pre>Write in plain words. Report to /var/lib/m8r/artifacts/1.\n\
                 Follow &lt;these&gt;:\n(nothing)</pre>"
            ),
            "{page}"
        );

        let res = request
            .post("/configurations/1")
            .form(&form("Evenings", ""))
            .await;
        assert_eq!(location(&res), "/configurations/1");
        let configuration = configurations::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(configuration.name, "Evenings");
        assert!(!configuration.active);
        let page = request.get("/configurations/1").await.text();
        assert!(
            page.contains("Blank: the schedules' prompts get nothing from this configuration."),
            "{page}"
        );

        assert_eq!(
            request
                .post("/configurations/1/activate")
                .await
                .status_code(),
            303
        );
        assert!(configurations::Model::active(&ctx.db)
            .await
            .unwrap()
            .is_some());
        assert!(request
            .get("/configurations")
            .await
            .text()
            .contains("Deactivate"));
        assert_eq!(
            request
                .post("/configurations/1/deactivate")
                .await
                .status_code(),
            303
        );
        assert!(configurations::Model::active(&ctx.db)
            .await
            .unwrap()
            .is_none());

        assert_eq!(
            request.post("/configurations/1/remove").await.status_code(),
            303
        );
        assert_eq!(request.get("/configurations/1").await.status_code(), 404);
        assert_eq!(
            request.post("/configurations/1/remove").await.status_code(),
            404
        );
    })
    .await;
}
