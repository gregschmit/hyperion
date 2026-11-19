//! The JSON API, as a caller with a token for everything sees it.

use loco_rs::{prelude::*, TestServer};
use m8r::{app::App, models::artifacts};
use serde_json::{json, Value};
use serial_test::serial;

use crate::support::{self, queue, start, token, ISSUES, REQUESTS};

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
    with_token(|request, ctx, token| async move {
        let items = "/api/watchlist_items";
        let res = request.get(items).authorization_bearer(&token).await;
        assert_eq!(answer(&res), (200, json!([])));
        let repo = support::repo(&ctx.db, "~/git/o/r", "https://github.com/o/r").await;
        let other = support::repo(&ctx.db, "~/git/g/p", "https://git.example.com/g/p").await;

        // As m8r's prompt adds one: its URL says which repo it's in, and what it is.
        let res = request
            .post(items)
            .authorization_bearer(&token)
            .json(
                &json!({ "url": PR_URL, "priority": "high", "note": "Ships on Friday",
                "requested": true }),
            )
            .await;
        let added = json!({
            "id": 1,
            "url": PR_URL,
            "repo_id": repo.id,
            "repo": "~/git/o/r",
            "category": REQUESTS,
            "priority": "high",
            "note": "Ships on Friday",
            "requested": true,
            "title": "",
            "checked_at": null,
        });
        assert_eq!(answer(&res), (200, added.clone()));
        // The URL is all that an item needs. A category that's sent says nothing: the URL does.
        let res = request
            .post(items)
            .authorization_bearer(&token)
            .json(&json!({ "url": ISSUE_URL, "category": "Epic" }))
            .await;
        let (status, issue) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (
                &issue["id"],
                &issue["category"],
                &issue["priority"],
                &issue["note"],
                &issue["requested"]
            ),
            (
                &json!(2),
                &json!(ISSUES),
                &json!("medium"),
                &json!(""),
                &json!(false)
            )
        );

        // What can't be saved comes back with the reason.
        for (body, why) in [
            (
                json!({ "url": "https://example.com/" }),
                "That isn't the URL of an issue or a PR/MR on GitHub or GitLab.",
            ),
            (
                json!({ "url": "https://github.com/o/unknown/issues/1" }),
                "m8r has no repo for https://github.com/o/unknown. Add that repo first: an item \
                 is in one.",
            ),
            (
                json!({ "url": ISSUE_URL, "priority": "low" }),
                "The priority is medium or high.",
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
            .json(&json!({ "url": PR_URL, "requested": true }))
            .await;
        let (_, again) = answer(&res);
        assert_eq!(
            (&again["id"], &again["priority"], &again["note"]),
            (&json!(1), &json!("medium"), &json!(""))
        );

        // A change names only what changes, as m8r's prompt does after a review.
        let res = request
            .patch(&format!("{items}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "requested": false }))
            .await;
        let (status, changed) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (&changed["requested"], &changed["category"], &changed["url"]),
            (&json!(false), &json!(REQUESTS), &json!(PR_URL))
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
        assert_eq!(answer(&res), (200, changed.clone()));
        // One category's items, one repo's, or both.
        let res = request
            .get(items)
            .add_query_param("category", ISSUES)
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!([issue])));
        let res = request
            .get(items)
            .add_query_param("repo_id", repo.id)
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!([changed, issue])));
        let res = request
            .get(items)
            .add_query_param("repo_id", other.id)
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!([])));
        let res = request
            .get(items)
            .add_query_param("repo_id", repo.id)
            .add_query_param("category", REQUESTS)
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!([changed])));
        let res = request
            .get(items)
            .add_query_param("category", "Epic")
            .authorization_bearer(&token)
            .await;
        assert_eq!(
            answer(&res),
            bad_request(
                "There's no category \"Epic\". The categories are: issues, pull/merge requests."
            )
        );

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
async fn repos_are_listed_added_changed_and_removed() {
    with_token(|request, _ctx, token| async move {
        let repos = "/api/repos";
        let here = std::env::current_dir().unwrap();
        let here = here.to_string_lossy();
        let (src, tests) = (format!("{here}/src"), format!("{here}/tests"));
        let res = request.get(repos).authorization_bearer(&token).await;
        assert_eq!(answer(&res), (200, json!([])));

        // A clone that's there, and its project, however that's written.
        let res = request
            .post(repos)
            .authorization_bearer(&token)
            .json(&json!({ "path": tests, "url": "git@github.com:o/r.git" }))
            .await;
        let added = json!({ "id": 1, "path": tests, "url": "https://github.com/o/r" });
        assert_eq!(answer(&res), (200, added.clone()));
        // With no URL, the clone's origin says the project. This one is no clone, so it has
        // none.
        let res = request
            .post(repos)
            .authorization_bearer(&token)
            .json(&json!({ "path": src }))
            .await;
        assert_eq!(
            answer(&res),
            (200, json!({ "id": 2, "path": src, "url": "" }))
        );

        for (body, why) in [
            (
                json!({ "path": src }),
                format!("m8r already has that clone: {src}."),
            ),
            (
                json!({ "path": format!("{here}/no-such-clone") }),
                format!(
                    "A repo is a clone in the home of m8r's user, such as ~/git/group/project, \
                     and \"{here}/no-such-clone\" isn't a directory there."
                ),
            ),
            (
                json!({ "path": format!("{here}/migration"), "url": "https://github.com/O/R/" }),
                format!(
                    "https://github.com/O/R is the project of the repo {tests}, and a project \
                     has one repo."
                ),
            ),
        ] {
            let res = request
                .post(repos)
                .authorization_bearer(&token)
                .json(&body)
                .await;
            assert_eq!(answer(&res), bad_request(&why), "{body}");
        }

        // A change names only what changes. The list goes by the clones' paths.
        let res = request
            .patch(&format!("{repos}/2"))
            .authorization_bearer(&token)
            .json(&json!({ "url": "https://git.example.com/g/p" }))
            .await;
        let changed = json!({ "id": 2, "path": src, "url": "https://git.example.com/g/p" });
        assert_eq!(answer(&res), (200, changed.clone()));
        let res = request
            .get(&format!("{repos}/2"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, changed.clone()));
        let res = request.get(repos).authorization_bearer(&token).await;
        assert_eq!(answer(&res), (200, json!([changed, added])));

        // A repo that an item is in stays.
        request
            .post("/api/watchlist_items")
            .authorization_bearer(&token)
            .json(&json!({ "url": ISSUE_URL }))
            .await;
        let res = request
            .delete(&format!("{repos}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(
            answer(&res),
            bad_request(&format!(
                "\"{tests}\" is in use: by 1 watchlist items and 0 schedules. Take them off it \
                 first."
            ))
        );
        request
            .delete("/api/watchlist_items/1")
            .authorization_bearer(&token)
            .await;
        let res = request
            .delete(&format!("{repos}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, json!({})));
        let res = request
            .delete(&format!("{repos}/1"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), not_found());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn schedules_are_listed_added_changed_run_and_removed() {
    with_token(|request, ctx, token| async move {
        let schedules = "/api/schedules";
        let here = std::env::current_dir().unwrap();
        let here = here.to_string_lossy();
        // Its repos are ones that m8r has, by their clones' paths. They and its times are
        // lists.
        let (src, tests) = (format!("{here}/src"), format!("{here}/tests"));
        let in_src = support::repo(&ctx.db, &src, "").await;
        let in_tests = support::repo(&ctx.db, &tests, "").await;
        let res = request
            .post(schedules)
            .authorization_bearer(&token)
            .json(
                &json!({ "name": "review", "category": REQUESTS, "max_items": 50,
                "repos": [tests, src], "when": ["0 6 * * 1-5", "every saturday at 10:00"],
                "command": "claude -p", "prompt": "{{ m8r_prompt }}\n\n{{ watchlist }}" }),
            )
            .await;
        let (status, added) = answer(&res);
        assert_eq!(status, 200, "{added}");
        // The times are in words, and the next one is a moment. A new schedule is enabled.
        let next = added["next"].as_str().unwrap().to_string();
        assert!(next.ends_with('Z') && next.len() == 20, "{next}");
        assert_eq!(
            added,
            json!({
                "id": 1,
                "name": "review",
                "category": REQUESTS,
                "max_items": 50,
                "repos": [src, tests],
                "when": ["every weekday at 06:00", "every saturday at 10:00"],
                "enabled": true,
                "command": "claude -p",
                "prompt": "{{ m8r_prompt }}\n\n{{ watchlist }}",
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

        for (body, why) in [
            (
                json!({ "name": "triage", "repos": [src], "when": ["every day at 6am"],
                "command": "claude -p" }),
                "error: unknown keyword 'am'\n  every day at 6am\n                ^^".to_string(),
            ),
            (
                json!({ "name": "review", "repos": [src], "when": ["every day at 6:00"],
                "command": "claude -p" }),
                "There's already a schedule \"review\".".to_string(),
            ),
            (
                json!({ "name": "triage", "repos": [here], "when": ["every day at 6:00"],
                "command": "claude -p" }),
                format!("\"{here}\" isn't one of m8r's repos. Add it to the repos first."),
            ),
        ] {
            let res = request
                .post(schedules)
                .authorization_bearer(&token)
                .json(&body)
                .await;
            assert_eq!(answer(&res), bad_request(&why), "{body}");
        }

        // A change names only what changes. A blank category is no items. A list of one can
        // be text, and so can a longer one, with a line for each.
        let res = request
            .patch(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "when": "every 12 hours", "category": "" }))
            .await;
        let (status, changed) = answer(&res);
        assert_eq!(status, 200);
        assert_eq!(
            (
                &changed["when"],
                &changed["category"],
                &changed["max_items"],
                &changed["command"],
                &changed["repos"]
            ),
            (
                &json!(["every 12 hours from 00:00 to 23:59"]),
                &json!(""),
                &json!(50),
                &json!("claude -p"),
                &json!([src, tests])
            )
        );
        let res = request
            .patch(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "repos": tests, "max_items": 0, "name": "evenings" }))
            .await;
        let (_, changed) = answer(&res);
        assert_eq!(
            (&changed["repos"], &changed["max_items"], &changed["name"]),
            (&json!([tests]), &json!(0), &json!("evenings"))
        );
        let res = request
            .patch(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "repos": format!("{tests}\n{src}") }))
            .await;
        assert_eq!(res.json::<Value>()["repos"], json!([src, tests]));
        // Off, it has no next time. On again, it has one.
        let switch = |enabled: bool| {
            let (request, token) = (&request, token.clone());
            async move {
                let res = request
                    .patch(&format!("{schedules}/1"))
                    .authorization_bearer(&token)
                    .json(&json!({ "enabled": enabled }))
                    .await;
                answer(&res).1
            }
        };
        let off = switch(false).await;
        assert_eq!(
            (&off["enabled"], &off["next"], &off["name"]),
            (&json!(false), &json!(null), &json!("evenings"))
        );
        let on = switch(true).await;
        assert_eq!(on["enabled"], json!(true));
        assert!(on["next"].is_string(), "{on}");
        let res = request
            .patch(&format!("{schedules}/1"))
            .authorization_bearer(&token)
            .json(&json!({ "repos": [] }))
            .await;
        assert_eq!(
            answer(&res),
            bad_request("Say which repos the schedule runs in.")
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

        // "Run now": the answer is the runs that wait, one for each repo, and again the same
        // ones.
        let res = request
            .post(&format!("{schedules}/1/run"))
            .authorization_bearer(&token)
            .await;
        let (status, runs) = answer(&res);
        assert_eq!(status, 200);
        let queued: Vec<_> = runs
            .as_array()
            .unwrap()
            .iter()
            .map(|run| {
                (
                    &run["id"],
                    &run["schedule_id"],
                    &run["name"],
                    &run["repo_id"],
                    &run["repo"],
                    &run["status"],
                    &run["command"],
                )
            })
            .collect();
        let (one, name) = (json!(1), json!("evenings"));
        let (queued_status, command) = (json!("queued"), json!("claude -p"));
        assert_eq!(
            queued,
            [
                (
                    &json!(1),
                    &one,
                    &name,
                    &json!(in_src.id),
                    &json!(src),
                    &queued_status,
                    &command
                ),
                (
                    &json!(2),
                    &one,
                    &name,
                    &json!(in_tests.id),
                    &json!(tests),
                    &queued_status,
                    &command
                ),
            ]
        );
        let res = request
            .post(&format!("{schedules}/1/run"))
            .authorization_bearer(&token)
            .await;
        assert_eq!(answer(&res), (200, runs));

        // A repo that a schedule runs in stays.
        let res = request
            .delete(&format!("/api/repos/{}", in_src.id))
            .authorization_bearer(&token)
            .await;
        assert_eq!(
            answer(&res),
            bad_request(&format!(
                "\"{src}\" is in use: by 0 watchlist items and 1 schedules. Take them off it \
                 first."
            ))
        );

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
        // Its times are moments, by the names of their columns.
        for time in ["created_at", "started_at", "finished_at"] {
            assert!(whole[time].as_str().unwrap().contains('T'), "{time}");
        }

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
