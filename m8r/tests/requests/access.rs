use loco_rs::testing::{
    prelude::*,
    request::{request_with_config, RequestConfigBuilder},
};
use m8r::app::App;
use serial_test::serial;

use crate::support::{link_path, location, signed_in};

fn keeps_cookies() -> loco_rs::testing::request::RequestConfig {
    RequestConfigBuilder::new().save_cookies(true).build()
}

#[tokio::test]
#[serial]
async fn nothing_answers_without_a_session() {
    request::<App, _, _>(|request, _ctx| async move {
        for path in [
            "/",
            "/runs",
            "/items",
            "/configurations/new",
            "/static/m8r.css",
            "/nope",
        ] {
            let res = request.get(path).await;
            assert_eq!(res.status_code(), 401, "{path}");
            assert!(res.text().contains("m8r task link"), "{path}");
        }
        let res = request.post("/items").form(&[("url", "x")]).await;
        assert_eq!(res.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn an_access_link_starts_a_session_any_number_of_times() {
    request_with_config::<App, _, _>(keeps_cookies(), |request, ctx| async move {
        let link = link_path(&ctx, 60, false);
        assert!(link.starts_with("/?token="), "{link}");

        let res = request.get(&link).await;
        assert_eq!(res.status_code(), 303);
        // The token leaves the address bar.
        assert_eq!(location(&res), "/");
        let cookie = res.header("set-cookie").to_str().unwrap().to_string();
        assert!(cookie.contains("HttpOnly"), "{cookie}");
        assert!(cookie.contains("SameSite=Lax"), "{cookie}");
        assert!(cookie.contains("Max-Age=28800"), "{cookie}");

        assert_eq!(request.get("/").await.status_code(), 200);
        assert_eq!(request.get("/static/m8r.css").await.status_code(), 200);
        // The same link works again, and keeps the rest of the address.
        let res = request
            .get(&format!("/runs{}&x=1", link.replacen('/', "", 1)))
            .await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/runs?x=1");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_single_use_link_works_one_time() {
    request::<App, _, _>(|request, ctx| async move {
        let link = link_path(&ctx, 60, true);

        assert_eq!(request.get(&link).await.status_code(), 303);
        let again = request.get(&link).await;
        assert_eq!(again.status_code(), 401);
        assert!(again.text().contains("works one time"), "{}", again.text());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_link_that_expired_or_was_never_good_is_refused() {
    request::<App, _, _>(|request, ctx| async move {
        // Past the signer's leeway for clocks that disagree.
        let expired = link_path(&ctx, 0, false);
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        for link in [expired.as_str(), "/?token=nonsense", "/?token="] {
            let res = request.get(link).await;
            assert_eq!(res.status_code(), 401, "{link}");
            assert!(res.maybe_header("set-cookie").is_none(), "{link}");
        }

        // A token signed with another secret.
        let good = link_path(&ctx, 60, false);
        let forged = format!("{}x", &good[..good.len() - 1]);
        assert_eq!(request.get(&forged).await.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_session_can_change_things() {
    signed_in(|request, _ctx| async move {
        let res = request
            .post("/items")
            .form(&[
                ("url", "https://github.com/o/r/issues/1"),
                ("priority", "high"),
                ("note", ""),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/items");
    })
    .await;
}
