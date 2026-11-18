use m8r::models::configurations;
use serial_test::serial;

use crate::support::{location, signed_in};

fn form<'a>(name: &'a str, prompt: &'a str) -> [(&'a str, &'a str); 2] {
    [("name", name), ("prompt", prompt)]
}

#[tokio::test]
#[serial]
async fn a_configuration_is_written_seen_and_made_active() {
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
        // The prompt is plain text, so the page names nothing for it to use: only how a
        // schedule's prompt places it.
        assert!(page.contains("<code>{{ config_prompt }}</code>"), "{page}");
        assert!(
            !page.contains("m8r_prompt") && !page.contains("{{ watchlist }}"),
            "{page}"
        );

        // A configuration has a name.
        let res = request.post("/configurations").form(&form(" ", "x")).await;
        assert_eq!(res.status_code(), 200);
        assert!(res.text().contains("Give the configuration a name."));
        assert!(configurations::Model::list(&ctx.db)
            .await
            .unwrap()
            .is_empty());

        // Any text is a prompt. It's kept and shown as written: what looks like a tag is
        // text, and so is what no template could hold.
        let written = "Write in plain words.\nFollow <these>: {{ watchlist }} {% for %}";
        let res = request
            .post("/configurations")
            .form(&form("Mornings", written))
            .await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/configurations/1");
        let configuration = configurations::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(configuration.prompt, written);
        let page = request.get("/configurations/1").await.text();
        assert!(
            page.contains(
                "rows=\"12\">Write in plain words.\n\
                 Follow &lt;these&gt;: {{ watchlist }} {% for %}</textarea>"
            ),
            "{page}"
        );
        // Nothing is compiled, so the page shows nothing compiled.
        assert!(!page.contains("Compiled"), "{page}");

        let res = request
            .post("/configurations/1")
            .form(&form("Evenings", ""))
            .await;
        assert_eq!(location(&res), "/configurations/1");
        let configuration = configurations::Model::find(&ctx.db, 1).await.unwrap();
        assert_eq!(
            (configuration.name.as_str(), configuration.prompt.as_str()),
            ("Evenings", "")
        );
        assert!(!configuration.active);

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
