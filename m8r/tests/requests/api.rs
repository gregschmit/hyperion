//! The JSON API, as a caller with a token for everything sees it.

use loco_rs::{prelude::*, TestServer};
use m8r::{app::App, models::artifacts};
use serde_json::{json, Value};
use serial_test::serial;

use crate::support::{queue, start, token, ISSUE, PR};

const PR_URL: &str = "https://github.com/o/r/pull/7";
const ISSUE_URL: &str = "https://github.com/o/r/issues/8";

/// Run a test as the holder of a token for everything.
async fn with_token<F, Fut>(callback: F)
where
    F: FnOnce(TestServer, AppContext, String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    request::<App, _, _>(|request, ctx| async move {
        let token = token(&ctx, "test", "all").await;
        callback(request, ctx, token).await;
    })
    .await;
}

/// The status and the body of an answer.
fn answer(res: &axum_test::TestResponse) -> (u16, Value) {
    (res.status_code().as_u16(), res.json())
}

fn bad_request(why: &str) -> (u16, Value) {
    (400, json!({ "error": "Bad Request", "description": why }))
}

fn not_found() -> (u16, Value) {
    (
        404,
        json!({ "error": "not_found", "description": "Resource was not found" }),
    )
}

#[tokio::test]
#[serial]
async fn watchlist_items_are_listed_added_changed_and_removed() {
    with_token(|request, _ctx, token| async move {
        let items = "/api/watchlist_items";
        let res = request.get(items).authorization_bearer(&token).await;
        assert_eq!(answer(&res), (200, json!([])));

        // As m8r's skill adds one: the category by its name.
        let res = request
            .post(items)
            .authorization_bearer(&token)
            .json(&json!({ "url": PR_URL, "category": PR, "priority": "high",
                "note": "Ships on Friday", "review_requested": true }))
            .await;
        let added = json!({
            "id": 1,
            "url": PR_URL,
            "category": PR,
            "priority": "high",
            "note": "Ships on Friday",
            "review_requested": true,
            "title": "",
            "checked_at": null,
        });
        assert_eq!(answer(&res), (200, added.clone()));
        // The URL and the category are all that an item needs.
        let res = request
            .post(items)
            .authorization_bearer(&token)
            .json(&json!({ "url": ISSUE_URL, "category": ISSUE }))
            .await;
        let (status, issue) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (
                &issue["id"],
                &issue["priority"],
                &issue["note"],
                &issue["review_requested"]
            ),
            (&json!(2), &json!("medium"), &json!(""), &json!(false))
        );

        // What can't be saved comes back with the reason.
        for (body, why) in [
            (
                json!({ "url": ISSUE_URL, "category": "Epic" }),
                "There's no category \"Epic\". The categories are: Issue, Pull/Merge Request.",
            ),
            (
                json!({ "url": ISSUE_URL }),
                "Choose a category. The categories are: Issue, Pull/Merge Request.",
            ),
            (
                json!({ "url": "https://example.com/", "category": ISSUE }),
                "That isn't the URL of an issue or a PR/MR on GitHub or GitLab.",
            ),
            (
                json!({ "url": ISSUE_URL, "category": ISSUE, "review_requested": true }),
                "Only a PR/MR can have its review requested. That's an issue.",
            ),
        ] {
            let res = request
                .post(items)
                .authorization_bearer(&token)
                .json(&body)
                .await;
            assert_eq!(answer(&res), bad_request(why), "{body}");
        }

        // The same URL again changes the item that's there.
        let res = request
            .post(items)
            .authorization_bearer(&token)
            .json(&json!({ "url": PR_URL, "category": PR, "review_requested": true }))
            .await;
        let (_, again) = answer(&res);
        assert_eq!(
            (&again["id"], &again["priority"], &again["note"]),
            (&json!(1), &json!("medium"), &json!(""))
        );

        // A change names only what changes, as the skill does after a review.
        let res = request
            .patch(&format!("{items}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "review_requested": false }))
            .await;
        let (status, changed) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (
                &changed["review_requested"],
                &changed["category"],
                &changed["url"]
            ),
            (&json!(false), &json!(PR), &json!(PR_URL))
        );
        let res = request
            .patch(&format!("{items}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "priority": "low" }))
            .await;
        assert_eq!(answer(&res), bad_request("The priority is medium or high."));

        let res = request
            .get(&format!("{items}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, changed));
        // One category's items.
        let res = request
            .get(items)
            .add_query_param("category", ISSUE)
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!([issue])));
        let res = request
            .get(items)
            .add_query_param("category", "Epic")
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.status_code(), 400);

        let res = request
            .delete(&format!("{items}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!({})));
        for res in [
            request
                .delete(&format!("{items}/1"))
                .authorization_bearer(&token)
                .await,
            request
                .get(&format!("{items}/1"))
                .authorization_bearer(&token)
                .await,
            request
                .patch(&format!("{items}/1"))
                .authorization_bearer(&token)
                .json(&json!({}))
                .await,
        ] {
            assert_eq!(answer(&res), not_found());
        }
        let res = request.get(items).authorization_bearer(&token).await;
        assert_eq!(res.json::<Value>().as_array().unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn categories_are_listed_added_renamed_and_removed() {
    with_token(|request, _ctx, token| async move {
        let categories = "/api/categories";
        let res = request.get(categories).authorization_bearer(&token).await;
        assert_eq!(
            answer(&res),
            (
                200,
                json!([{ "id": 1, "name": ISSUE }, { "id": 2, "name": PR }])
            )
        );

        let res = request
            .post(categories)
            .authorization_bearer(&token)
            .json(&json!({ "name": "Epic" }))
            .await;
        assert_eq!(answer(&res), (200, json!({ "id": 3, "name": "Epic" })));
        let res = request
            .post(categories)
            .authorization_bearer(&token)
            .json(&json!({ "name": "Epic" }))
            .await;
        assert_eq!(
            answer(&res),
            bad_request("There's already a category \"Epic\".")
        );

        let res = request
            .patch(&format!("{categories}/3"))
            .authorization_bearer(&token)
            .json(&json!({ "name": "Release" }))
            .await;
        assert_eq!(answer(&res), (200, json!({ "id": 3, "name": "Release" })));
        let res = request
            .get(&format!("{categories}/3"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!({ "id": 3, "name": "Release" })));

        // A category that an item is in stays.
        request
            .post("/api/watchlist_items")
            .authorization_bearer(&token)
            .json(&json!({ "url": ISSUE_URL, "category": "Release" }))
            .await;
        let res = request
            .delete(&format!("{categories}/3"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(
            answer(&res),
            bad_request(
                "\"Release\" is in use: by 1 watchlist items and 0 schedules. Give them another \
                 category first."
            )
        );
        request
            .patch("/api/watchlist_items/1")
            .authorization_bearer(&token)
            .json(&json!({ "category": ISSUE }))
            .await;
        let res = request
            .delete(&format!("{categories}/3"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!({})));
        let res = request
            .delete(&format!("{categories}/3"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), not_found());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn schedules_are_listed_added_changed_run_and_removed() {
    with_token(|request, _ctx, token| async move {
        let schedules = "/api/schedules";
        let here = std::env::current_dir().unwrap();
        let here = here.to_string_lossy();
        let res = request
            .post(schedules)
            .authorization_bearer(&token)
            .json(
                &json!({ "operation": "review", "category": PR, "repo": here,
                "hron": "0 6 * * 1-5", "command": "claude -p",
                "prompt": "{{ system_message }}\n\n{{ watchlist }}" }),
            )
            .await;
        let (status, added) = answer(&res);
        assert_eq!(status, 200, "{added}");
        // The time is in words, and the next one is a moment.
        let next = added["next"].as_str().unwrap().to_string();
        assert!(next.ends_with('Z') && next.len() == 20, "{next}");
        assert_eq!(
            added,
            json!({
                "id": 1,
                "operation": "review",
                "category": PR,
                "repo": here,
                "hron": "every weekday at 06:00",
                "command": "claude -p",
                "prompt": "{{ system_message }}\n\n{{ watchlist }}",
                "next": next,
            })
        );
        let res = request.get(schedules).authorization_bearer(&token).await;
        assert_eq!(answer(&res), (200, json!([added.clone()])));
        let res = request
            .get(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, added));

        let res = request
            .post(schedules)
            .authorization_bearer(&token)
            .json(
                &json!({ "operation": "review", "repo": here, "hron": "every day at 6am",
                "command": "claude -p" }),
            )
            .await;
        assert_eq!(
            answer(&res),
            bad_request("error: unknown keyword 'am'\n  every day at 6am\n                ^^")
        );

        // A change names only what changes. A blank category is none.
        let res = request
            .patch(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "hron": "every 12 hours", "category": "" }))
            .await;
        let (status, changed) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (&changed["hron"], &changed["category"], &changed["command"]),
            (
                &json!("every 12 hours from 00:00 to 23:59"),
                &json!(""),
                &json!("claude -p")
            )
        );
        let res = request
            .patch(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "prompt": "{{ nope }}" }))
            .await;
        let (status, refused) = answer(&res);
        assert_eq!(status, 400);
        let why = refused["description"].as_str().unwrap();
        assert!(why.starts_with("The prompt doesn't compile."), "{why}");

        // "Run now": the answer is the run that waits, and again the same one.
        let res = request
            .post(&format!("{schedules}/1/run"))
            .authorization_bearer(&token)
            .await;
        let (status, run) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (
                &run["id"],
                &run["schedule_id"],
                &run["status"],
                &run["command"]
            ),
            (&json!(1), &json!(1), &json!("queued"), &json!("claude -p"))
        );
        let res = request
            .post(&format!("{schedules}/1/run"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.json::<Value>()["id"], json!(1));

        let res = request
            .delete(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!({})));
        let res = request
            .post(&format!("{schedules}/1/run"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), not_found());
        // The run stays in the history, with no schedule.
        let res = request
            .get("/api/runs/1")
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.json::<Value>()["schedule_id"], json!(null));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn configurations_are_listed_added_changed_made_active_and_removed() {
    with_token(|request, _ctx, token| async move {
        let configurations = "/api/configurations";
        let res = request
            .post(configurations)
            .authorization_bearer(&token)
            .json(
                &json!({ "name": "Mornings", "system_message": "Report to {{ artifacts_dir }}." }),
            )
            .await;
        let one = json!({
            "id": 1,
            "name": "Mornings",
            "active": false,
            "system_message": "Report to {{ artifacts_dir }}.",
        });
        assert_eq!(answer(&res), (200, one.clone()));
        let res = request
            .post(configurations)
            .authorization_bearer(&token)
            .json(&json!({ "name": "Evenings" }))
            .await;
        assert_eq!(res.json::<Value>()["system_message"], json!(""));
        let res = request
            .post(configurations)
            .authorization_bearer(&token)
            .json(&json!({ "system_message": "x" }))
            .await;
        assert_eq!(answer(&res), bad_request("Give the configuration a name."));

        let res = request
            .patch(&format!("{configurations}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "system_message": "{{ nope }}" }))
            .await;
        assert_eq!(res.status_code(), 400);
        let res = request
            .patch(&format!("{configurations}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "name": "Weekdays" }))
            .await;
        let (_, renamed) = answer(&res);
        assert_eq!(
            (&renamed["name"], &renamed["system_message"]),
            (&json!("Weekdays"), &one["system_message"])
        );

        // One is active, or none.
        let active = |id: u8| {
            let request = &request;
            let token = token.clone();
            async move {
                let path = format!("{configurations}/{id}");
                let res = request.get(&path).authorization_bearer(&token).await;
                res.json::<Value>()["active"].as_bool().unwrap()
            }
        };
        let res = request
            .post(&format!("{configurations}/1/activate"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.json::<Value>()["active"], json!(true));
        request
            .post(&format!("{configurations}/2/activate"))
            .authorization_bearer(&token)
            .await;
        assert_eq!((active(1).await, active(2).await), (false, true));
        // The active one comes first.
        let res = request
            .get(configurations)
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.json::<Value>()[0]["name"], json!("Evenings"));
        let res = request
            .post(&format!("{configurations}/2/deactivate"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.json::<Value>()["active"], json!(false));

        let res = request
            .delete(&format!("{configurations}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!({})));
        let res = request
            .post(&format!("{configurations}/1/activate"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), not_found());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn runs_are_listed_and_shown_with_their_artifacts() {
    with_token(|request, ctx, token| async move {
        let first = queue(&ctx.db, "review", "/home/m8r/git/rxg", "claude -p").await;
        let first = start(&ctx.db, first, "The prompt.").await;
        for (name, url) in [
            (
                "comment",
                "https://git.example.com/g/p/-/merge_requests/1#note_5",
            ),
            ("review", ""),
        ] {
            let mut artifact = artifacts::ActiveModel::new();
            artifact.run_id = Set(first.id);
            artifact.name = Set(name.to_string());
            artifact.url = Set(url.to_string());
            artifact.insert(&ctx.db).await.unwrap();
        }
        let first = first
            .into_active_model()
            .finished(
                &ctx.db,
                m8r::models::runs::Status::Ok,
                "What it printed.".to_string(),
                String::new(),
            )
            .await
            .unwrap();
        queue(&ctx.db, "triage", "/home/m8r/git/rxg", "claude -p").await;

        // Newest first, and no more than asked for.
        let res = request.get("/api/runs").authorization_bearer(&token).await;
        let (status, runs) = answer(&res);
        assert_eq!(status, 200);
        let ids: Vec<_> = runs
            .as_array()
            .unwrap()
            .iter()
            .map(|run| &run["id"])
            .collect();
        assert_eq!(ids, [&json!(2), &json!(1)]);
        let res = request
            .get("/api/runs")
            .add_query_param("limit", 1)
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.json::<Value>().as_array().unwrap().len(), 1);

        // A list leaves out the prompt and what the run printed. One run has them.
        let listed = &runs[1];
        assert_eq!(listed.get("prompt"), None);
        assert_eq!(listed.get("output"), None);
        assert_eq!(listed["status"], json!("ok"));
        assert_eq!(
            listed["artifacts"],
            json!([
                {
                    "id": 1,
                    "name": "comment",
                    "url": "https://git.example.com/g/p/-/merge_requests/1#note_5",
                    "hosted": false,
                },
                // Files that m8r hosts: the link is their page.
                {
                    "id": 2,
                    "name": "review",
                    "url": "http://localhost:5150/artifacts/2",
                    "hosted": true,
                },
            ])
        );
        let res = request
            .get(&format!("/api/runs/{}", first.id))
            .authorization_bearer(&token)
            .await;
        let (status, whole) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (&whole["prompt"], &whole["output"], &whole["artifacts"]),
            (
                &json!("The prompt."),
                &json!("What it printed."),
                &listed["artifacts"]
            )
        );
        assert!(whole["started_at"].as_str().unwrap().contains('T'));

        let res = request
            .get("/api/runs/9")
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), not_found());
        // The history isn't the API's to change.
        let res = request
            .delete("/api/runs/1")
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.status_code(), 405);
    })
    .await;
}
