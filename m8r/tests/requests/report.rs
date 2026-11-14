use chrono::{Days, Local};
use loco_rs::prelude::*;
use m8r::models::{
    artifacts,
    configurations::{self, Operation, Params},
    runs,
};
use serial_test::serial;

use crate::support::signed_in;

#[tokio::test]
#[serial]
async fn a_new_install_says_how_to_set_it_up() {
    signed_in(|request, ctx| async move {
        let res = request.get("/").await;
        assert_eq!(res.status_code(), 200);
        let page = res.text();
        assert!(page.contains("has nothing to do yet"), "{page}");
        assert!(page.contains("sudo -iu m8r"), "{page}");

        let params = Params {
            name: "One".to_string(),
            ..Params::starter()
        };
        configurations::Model::create(&ctx.db, &params)
            .await
            .unwrap();
        let page = request.get("/").await.text();
        assert!(!page.contains("has nothing to do yet"), "{page}");
        assert!(
            page.contains("No run reported an artifact on this day."),
            "{page}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_report_links_to_the_days_artifacts() {
    signed_in(|request, ctx| async move {
        let run = runs::Model::queue(&ctx.db, Operation::Review, "/home/m8r/git/rxg")
            .await
            .unwrap()
            .unwrap();
        for (name, url) in [
            ("comment on rxg!1", "https://git.example.com/g/p/-/merge_requests/1#note_5"),
            ("review <of> rxg!2", ""),
        ] {
            let mut artifact = artifacts::ActiveModel::new();
            artifact.run_id = Set(run.id);
            artifact.name = Set(name.to_string());
            artifact.url = Set(url.to_string());
            artifact.insert(&ctx.db).await.unwrap();
        }

        let page = request.get("/").await.text();
        assert!(page.contains("(today)"), "{page}");
        assert!(page.contains(r#"<a href="/runs/1">review</a>"#), "{page}");
        assert!(
            page.contains(
                r#"<a href="https://git.example.com/g/p/-/merge_requests/1#note_5">comment on rxg!1</a>"#
            ),
            "{page}"
        );
        // A hosted one links to m8r's copy, and its name is escaped.
        assert!(
            page.contains(r#"<a href="/artifacts/2">review &lt;of&gt; rxg!2</a>"#),
            "{page}"
        );

        // Another day has none of them.
        let yesterday = Local::now().date_naive().checked_sub_days(Days::new(1)).unwrap();
        let page = request.get(&format!("/?day={yesterday}")).await.text();
        assert!(page.contains("No run reported an artifact on this day."), "{page}");
        assert!(!page.contains("comment on rxg!1"), "{page}");

        assert_eq!(request.get("/?day=someday").await.status_code(), 400);
    })
    .await;
}
