use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use loco_rs::{
    prelude::IntoActiveModel,
    testing::{
        prelude::*,
        request::{request_with_config, RequestConfigBuilder},
    },
};
use m8r::{app::App, initializers::access, models::runs};
use serial_test::serial;

use crate::support::{link_path, location, queue, signed_in, start, token};

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
            "/watchlist",
            "/schedules/new",
            "/static/m8r.css",
            "/nope",
        ] {
            let res = request.get(path).await;
            assert_eq!(res.status_code(), 401, "{path}");
            assert!(res.text().contains("m8r task link"), "{path}");
        }
        let res = request.post("/watchlist").form(&[("url", "x")]).await;
        assert_eq!(res.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn an_access_link_starts_a_session_any_number_of_times() {
    request_with_config::<App, _, _>(keeps_cookies(), |request, ctx| async move {
        let link = link_path(&ctx, 60, false);
        // The token is short enough to read: 39 characters that an address takes as they are.
        let token = link.strip_prefix("/?token=").unwrap();
        assert_eq!(token.len(), 39, "{link}");
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{link}"
        );

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
        // A link's time is in whole seconds, so wait one out.
        let expired = link_path(&ctx, 0, false);
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        for link in [expired.as_str(), "/?token=nonsense", "/?token="] {
            let res = request.get(link).await;
            assert_eq!(res.status_code(), 401, "{link}");
            assert!(res.maybe_header("set-cookie").is_none(), "{link}");
        }

        // A token that m8r's secret didn't seal.
        let good = link_path(&ctx, 60, true);
        let forged = format!("{}x", &good[..good.len() - 1]);
        assert_eq!(request.get(&forged).await.status_code(), 401);
        // A token that someone changed, to work more than once or for longer: its first byte
        // says whether it works one time, and the next four say until when. The seal no
        // longer fits what it says.
        let token = good.strip_prefix("/?token=").unwrap();
        let sealed = URL_SAFE_NO_PAD.decode(token).unwrap();
        assert_eq!((sealed.len(), sealed[0]), (29, 1));
        for (at, value) in [(0, 0), (1, 0xff), (28, sealed[28] ^ 1)] {
            let mut changed = sealed.clone();
            changed[at] = value;
            let forged = format!("/?token={}", URL_SAFE_NO_PAD.encode(&changed));
            assert_eq!(request.get(&forged).await.status_code(), 401, "byte {at}");
        }
        // Too short and too long are no tokens either.
        for cut in [&sealed[..28], &[sealed.as_slice(), &[0]].concat()[..]] {
            let forged = format!("/?token={}", URL_SAFE_NO_PAD.encode(cut));
            assert_eq!(request.get(&forged).await.status_code(), 401);
        }
        // The one that m8r made still works, one time.
        assert_eq!(request.get(&good).await.status_code(), 303);
        assert_eq!(request.get(&good).await.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_session_can_change_things() {
    signed_in(|request, ctx| async move {
        crate::support::repo(&ctx.db, "~/git/o/r", "https://github.com/o/r").await;
        let res = request
            .post("/watchlist")
            .form(&[
                ("url", "https://github.com/o/r/issues/1"),
                ("priority", "high"),
                ("note", ""),
            ])
            .await;
        assert_eq!(res.status_code(), 303);
        assert_eq!(location(&res), "/watchlist");
    })
    .await;
}

/// What the API and the MCP endpoint say to a request that they turn away.
fn refusal(res: &axum_test::TestResponse) -> (u16, String) {
    let body: serde_json::Value = res.json();
    let why = body["description"].as_str().unwrap_or_default().to_string();
    (res.status_code().as_u16(), why)
}

#[tokio::test]
#[serial]
async fn the_api_and_the_mcp_endpoint_want_a_token_and_never_take_a_session() {
    signed_in(|request, ctx| async move {
        // The session that opens the pages opens neither.
        assert_eq!(request.get("/watchlist").await.status_code(), 200);
        for path in [
            "/api/watchlist_items",
            "/api/runs",
            "/api",
            "/api/nope",
            "/mcp",
        ] {
            let res = request.get(path).await;
            assert_eq!(
                refusal(&res),
                (
                    401,
                    "Send a token: \"Authorization: Bearer <token>\".".to_string()
                ),
                "{path}"
            );
            assert_eq!(res.header("www-authenticate"), "Bearer", "{path}");
        }
        let res = request.post("/mcp").json(&serde_json::json!({})).await;
        assert_eq!(res.status_code(), 401);
        let res = request.delete("/api/watchlist_items/1").await;
        assert_eq!(res.status_code(), 401);

        let token = token(&ctx, "mac", "all").await;
        let res = request
            .get("/api/watchlist_items")
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.status_code(), 200);
        assert_eq!(res.text(), "[]");
        // Only "Bearer" carries a token.
        let res = request
            .get("/api/watchlist_items")
            .add_header("authorization", format!("Basic {token}"))
            .await;
        assert_eq!(res.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_token_opens_no_page() {
    request::<App, _, _>(|request, ctx| async move {
        let token = token(&ctx, "mac", "all").await;
        for path in ["/", "/watchlist", "/schedules/new", "/static/m8r.css"] {
            let res = request.get(path).authorization_bearer(&token).await;
            assert_eq!(res.status_code(), 401, "{path}");
            assert!(res.text().contains("m8r task link"), "{path}");
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_token_covers_what_its_scope_names() {
    request::<App, _, _>(|request, ctx| async move {
        let duet = token(&ctx, "duet", "watchlist_items").await;
        let both = token(&ctx, "both", "runs,repos").await;
        let mac = token(&ctx, "mac", "all").await;
        let status = |path: &'static str, token: String| {
            let request = &request;
            async move {
                let res = request.get(path).authorization_bearer(token).await;
                res.status_code().as_u16()
            }
        };
        for (path, covered_by_duet, covered_by_both) in [
            ("/api/watchlist_items", true, false),
            ("/api/repos", false, true),
            ("/api/schedules", false, false),
            ("/api/runs", false, true),
        ] {
            let expected = |covered: bool| if covered { 200 } else { 403 };
            assert_eq!(
                status(path, duet.clone()).await,
                expected(covered_by_duet),
                "{path}"
            );
            assert_eq!(
                status(path, both.clone()).await,
                expected(covered_by_both),
                "{path}"
            );
            assert_eq!(status(path, mac.clone()).await, 200, "{path}");
        }
        let res = request
            .get("/api/schedules/1")
            .authorization_bearer(&duet)
            .await;
        assert_eq!(
            refusal(&res),
            (403, "The token duet doesn't cover this.".to_string())
        );
        // What isn't a resource is nothing that a scope names.
        assert_eq!(status("/api", duet.clone()).await, 403);
        assert_eq!(status("/api/watchlist_items_too", duet.clone()).await, 403);
        assert_eq!(status("/api/nope", mac.clone()).await, 404);

        // A token that m8r never made, and one that it no longer keeps.
        let res = request
            .get("/api/runs")
            .authorization_bearer("m8r_nope")
            .await;
        assert_eq!(
            refusal(&res),
            (
                401,
                "That token isn't m8r's, or it was revoked.".to_string()
            )
        );
        assert!(m8r::models::tokens::Model::revoke(&ctx.db, "mac")
            .await
            .unwrap());
        assert_eq!(status("/api/runs", mac).await, 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_runs_token_works_while_the_run_does_and_covers_the_watchlist() {
    request::<App, _, _>(|request, ctx| async move {
        let run = queue(&ctx.db, "review", "/a", "cat").await;
        let token = access::run_token(&ctx, run.id, 60).unwrap();
        let items = |token: String| {
            let request = &request;
            async move {
                request
                    .get("/api/watchlist_items")
                    .authorization_bearer(token)
                    .await
            }
        };

        // Not yet: the run waits.
        assert_eq!(
            refusal(&items(token.clone()).await),
            (401, "That run is over, and so is its token.".to_string())
        );
        let run = start(&ctx.db, run, "p").await;
        assert_eq!(items(token.clone()).await.status_code(), 200);
        let res = request
            .get("/api/schedules")
            .authorization_bearer(&token)
            .await;
        assert_eq!(
            refusal(&res),
            (403, format!("The token run {} doesn't cover this.", run.id))
        );

        run.into_active_model()
            .finished(&ctx.db, runs::Status::Ok, String::new(), String::new())
            .await
            .unwrap();
        assert_eq!(items(token).await.status_code(), 401);

        // A token for a run that isn't there, one that expired, and an access link's.
        let nowhere = access::run_token(&ctx, 999, 60).unwrap();
        assert_eq!(items(nowhere).await.status_code(), 401);
        let link = link_path(&ctx, 60, false);
        let link = link.split_once("token=").unwrap().1.to_string();
        assert_eq!(
            refusal(&items(link).await),
            (401, "That token isn't m8r's, or it expired.".to_string())
        );
        assert_eq!(items("not.a.jwt".to_string()).await.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_runs_token_starts_no_session() {
    request_with_config::<App, _, _>(keeps_cookies(), |request, ctx| async move {
        let run = queue(&ctx.db, "review", "/a", "cat").await;
        let run = start(&ctx.db, run, "p").await;
        let token = access::run_token(&ctx, run.id, 60).unwrap();

        let res = request.get(&format!("/?token={token}")).await;
        assert_eq!(res.status_code(), 401);
        assert_eq!(request.get("/").await.status_code(), 401);
    })
    .await;
}
