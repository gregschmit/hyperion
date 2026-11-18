//! The MCP endpoint, as a client such as Claude Duet sees it.

use loco_rs::{testing::prelude::*, TestServer};
use m8r::app::App;
use serde_json::{json, Value};
use serial_test::serial;

use crate::support::{configure, token, watch, ISSUE, PR};

/// Send one JSON-RPC message, as an MCP client does.
async fn send(request: &TestServer, token: &str, message: &Value) -> axum_test::TestResponse {
    request
        .post("/mcp")
        .authorization_bearer(token)
        .add_header("accept", "application/json, text/event-stream")
        .json(message)
        .await
}

/// The result of a JSON-RPC request.
async fn rpc(request: &TestServer, token: &str, method: &str, params: Value) -> Value {
    let message = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let res = send(request, token, &message).await;
    assert_eq!(res.status_code(), 200, "{}", res.text());
    assert_eq!(res.header("content-type"), "application/json");
    res.json()
}

/// The tools that a token's holder sees, by name.
async fn tools(request: &TestServer, token: &str) -> Vec<Value> {
    let listed = rpc(request, token, "tools/list", json!({})).await;
    listed["result"]["tools"].as_array().unwrap().clone()
}

/// Call a tool. Ok is what it gives back, and Err is what it tells the model went wrong.
async fn call(
    request: &TestServer,
    token: &str,
    name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let params = json!({ "name": name, "arguments": arguments });
    let answer = rpc(request, token, "tools/call", params).await;
    let result = &answer["result"];
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{answer}"));
    if result["isError"] == json!(true) {
        return Err(text.to_string());
    }
    Ok(serde_json::from_str(text).unwrap())
}

#[tokio::test]
#[serial]
async fn a_client_starts_with_no_session_and_reads_what_m8r_is() {
    request::<App, _, _>(|request, ctx| async move {
        let duet = token(&ctx, "duet", "watchlist_items").await;
        let hello = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" },
            },
        });

        let res = send(&request, &duet, &hello).await;
        assert_eq!(res.status_code(), 200);
        // Nothing to keep between two requests.
        assert!(res.maybe_header("mcp-session-id").is_none());
        let result = &res.json::<Value>()["result"];
        assert_eq!(result["protocolVersion"], json!("2025-06-18"));
        assert_eq!(result["capabilities"], json!({ "tools": {} }));
        assert_eq!(result["serverInfo"]["name"], json!("m8r"));
        let instructions = result["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("keeps the developer's watchlist"),
            "{instructions}"
        );

        let ready = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(send(&request, &duet, &ready).await.status_code(), 202);
        // And the first request can be any: here, after a restart.
        assert_eq!(tools(&request, &duet).await.len(), 4);

        // No stream to listen on, and no way in without a token.
        let res = request
            .get("/mcp")
            .authorization_bearer(&duet)
            .add_header("accept", "text/event-stream")
            .await;
        assert_eq!(res.status_code(), 405);
        let res = request
            .post("/mcp")
            .add_header("accept", "application/json, text/event-stream")
            .json(&hello)
            .await;
        assert_eq!(res.status_code(), 401);
        assert_eq!(res.header("www-authenticate"), "Bearer");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_token_sees_the_tools_of_what_it_covers() {
    request::<App, _, _>(|request, ctx| async move {
        let duet = token(&ctx, "duet", "watchlist_items").await;
        let brief = token(&ctx, "brief", "runs,categories").await;
        let mac = token(&ctx, "mac", "all").await;
        let names = |tools: &[Value]| {
            let names = tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap().to_string());
            names.collect::<Vec<_>>()
        };

        // Duet's only job is the watchlist.
        let for_duet = tools(&request, &duet).await;
        assert_eq!(
            names(&for_duet),
            [
                "list_watchlist_items",
                "add_watchlist_item",
                "update_watchlist_item",
                "remove_watchlist_item",
            ]
        );
        assert_eq!(
            names(&tools(&request, &brief).await),
            [
                "list_categories",
                "add_category",
                "rename_category",
                "remove_category",
                "list_runs",
                "get_run",
            ]
        );
        let all = tools(&request, &mac).await;
        assert_eq!(all.len(), 21);

        // Each tool says what it is, what it takes, and what it does to m8r.
        for tool in &all {
            let name = tool["name"].as_str().unwrap();
            assert!(name.len() <= 64, "{name}");
            assert!(!tool["title"].as_str().unwrap().is_empty(), "{name}");
            assert!(
                tool["description"].as_str().unwrap().ends_with('.'),
                "{name}"
            );
            assert_eq!(tool["inputSchema"]["type"], json!("object"), "{name}");
            let hints = &tool["annotations"];
            let reads = name.starts_with("list_") || name.starts_with("get_");
            assert_eq!(hints["readOnlyHint"], json!(reads), "{name}");
            if !reads {
                let removes = name.starts_with("remove_");
                assert_eq!(hints["destructiveHint"], json!(removes), "{name}");
            }
        }

        // The schema offers the categories that there are now.
        let add = &for_duet[1]["inputSchema"];
        assert_eq!(add["required"], json!(["url", "category"]));
        assert_eq!(add["properties"]["category"]["enum"], json!([ISSUE, PR]));
        assert_eq!(
            add["properties"]["priority"]["enum"],
            json!(["medium", "high"])
        );
        call(&request, &mac, "add_category", json!({ "name": "Epic" }))
            .await
            .unwrap();
        let for_duet = tools(&request, &duet).await;
        assert_eq!(
            for_duet[1]["inputSchema"]["properties"]["category"]["enum"],
            json!([ISSUE, PR, "Epic"])
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn duet_keeps_the_watchlist_with_its_tools() {
    request::<App, _, _>(|request, ctx| async move {
        let duet = token(&ctx, "duet", "watchlist_items").await;
        watch(&ctx, "https://github.com/o/r/issues/1", ISSUE).await;
        let mr = "https://git.example.com/g/p/-/merge_requests/9";

        // Someone asks for a review in a chat.
        let asked = json!({ "url": mr, "category": PR, "review_requested": true,
            "note": "Bob asked in chat" });
        let added = call(&request, &duet, "add_watchlist_item", asked)
            .await
            .unwrap();
        assert_eq!(
            added,
            json!({
                "id": 2,
                "url": mr,
                "category": PR,
                "priority": "medium",
                "note": "Bob asked in chat",
                "review_requested": true,
                "title": "",
                "checked_at": null,
            })
        );
        // It gets urgent.
        let urgent = json!({ "id": 2, "priority": "high" });
        let changed = call(&request, &duet, "update_watchlist_item", urgent)
            .await
            .unwrap();
        assert_eq!(
            (
                &changed["priority"],
                &changed["note"],
                &changed["review_requested"]
            ),
            (&json!("high"), &json!("Bob asked in chat"), &json!(true))
        );

        let listed = call(&request, &duet, "list_watchlist_items", json!({}))
            .await
            .unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 2);
        let prs = json!({ "category": PR });
        let listed = call(&request, &duet, "list_watchlist_items", prs)
            .await
            .unwrap();
        assert_eq!(listed, json!([changed]));

        // What the model got wrong, it can read and put right.
        for (name, arguments, why) in [
            (
                "add_watchlist_item",
                json!({ "url": mr, "category": "Epic" }),
                "There's no category \"Epic\". The categories are: Issue, Pull/Merge Request.",
            ),
            (
                "add_watchlist_item",
                json!({ "url": "https://example.com/", "category": PR }),
                "That isn't the URL of an issue or a PR/MR on GitHub or GitLab.",
            ),
            (
                "update_watchlist_item",
                json!({ "id": 1, "review_requested": true }),
                "Only a PR/MR can have its review requested. That's an issue.",
            ),
            (
                "remove_watchlist_item",
                json!({ "id": 99 }),
                "m8r has nothing with that id.",
            ),
            (
                "remove_watchlist_item",
                json!({}),
                "m8r can't read the arguments: missing field `id`.",
            ),
        ] {
            let refused = call(&request, &duet, name, arguments.clone()).await;
            assert_eq!(refused, Err(why.to_string()), "{name} {arguments}");
        }

        // The work is done.
        let removed = call(&request, &duet, "remove_watchlist_item", json!({ "id": 2 })).await;
        assert_eq!(removed, Ok(json!({ "removed": 2 })));
        let listed = call(&request, &duet, "list_watchlist_items", json!({}))
            .await
            .unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);

        // A tool that its token doesn't cover is no tool at all.
        for name in ["list_schedules", "remove_category", "no_such_tool"] {
            let params = json!({ "name": name, "arguments": {} });
            let answer = rpc(&request, &duet, "tools/call", params).await;
            assert_eq!(
                answer["error"],
                json!({ "code": -32602, "message": format!("Unknown tool: {name}") }),
                "{name}"
            );
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_other_tools_do_what_the_api_does() {
    request::<App, _, _>(|request, ctx| async move {
        let mac = token(&ctx, "mac", "all").await;
        let here = std::env::current_dir().unwrap();
        let here = here.to_string_lossy();
        configure(&ctx, "").await;

        // Categories.
        let epic = call(&request, &mac, "add_category", json!({ "name": "Epic" })).await;
        assert_eq!(epic, Ok(json!({ "id": 3, "name": "Epic" })));
        let renamed = json!({ "id": 3, "name": "Release" });
        let release = call(&request, &mac, "rename_category", renamed).await;
        assert_eq!(release, Ok(json!({ "id": 3, "name": "Release" })));
        let listed = call(&request, &mac, "list_categories", json!({}))
            .await
            .unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 3);

        // A schedule that takes that category, so the category stays.
        let asked = json!({ "operation": "other", "category": "Release", "repos": [here],
            "hron": ["every 12 hours", "every day at 09:00, 17:00"],
            "command": "make release-notes" });
        let schedule = call(&request, &mac, "add_schedule", asked).await.unwrap();
        assert_eq!(
            (
                &schedule["id"],
                &schedule["hron"],
                &schedule["category"],
                &schedule["prompt"]
            ),
            (
                &json!(1),
                &json!([
                    "every 12 hours from 00:00 to 23:59",
                    "every day at 09:00, 17:00"
                ]),
                &json!("Release"),
                &json!("")
            )
        );
        assert_eq!(schedule["repos"], json!([here]));
        let in_use = call(&request, &mac, "remove_category", json!({ "id": 3 })).await;
        assert!(in_use
            .unwrap_err()
            .contains("by 0 watchlist items and 1 schedules"));
        let every = json!({ "id": 1, "category": "", "prompt": "{{ watchlist }}" });
        let changed = call(&request, &mac, "update_schedule", every)
            .await
            .unwrap();
        assert_eq!(
            (
                &changed["category"],
                &changed["prompt"],
                &changed["command"]
            ),
            (
                &json!(""),
                &json!("{{ watchlist }}"),
                &json!("make release-notes")
            )
        );
        let gone = call(&request, &mac, "remove_category", json!({ "id": 3 })).await;
        assert_eq!(gone, Ok(json!({ "removed": 3 })));
        let bad = json!({ "id": 1, "hron": "whenever" });
        assert!(call(&request, &mac, "update_schedule", bad).await.is_err());
        let listed = call(&request, &mac, "list_schedules", json!({}))
            .await
            .unwrap();
        assert_eq!(listed, json!([changed]));

        // Its runs, one for each repo, and the history.
        let runs = call(&request, &mac, "run_schedule", json!({ "id": 1 }))
            .await
            .unwrap();
        let run = &runs[0];
        assert_eq!(runs.as_array().unwrap().len(), 1);
        assert_eq!(
            (
                &run["id"],
                &run["status"],
                &run["schedule_id"],
                &run["repo"]
            ),
            (&json!(1), &json!("queued"), &json!(1), &json!(here))
        );
        assert_eq!(run.get("prompt"), None);
        let listed = call(&request, &mac, "list_runs", json!({ "limit": 5 }))
            .await
            .unwrap();
        assert_eq!(listed, runs);
        let whole = call(&request, &mac, "get_run", json!({ "id": 1 }))
            .await
            .unwrap();
        assert_eq!(
            (&whole["prompt"], &whole["output"]),
            (&json!(""), &json!(""))
        );
        let removed = call(&request, &mac, "remove_schedule", json!({ "id": 1 })).await;
        assert_eq!(removed, Ok(json!({ "removed": 1 })));
        let missing = call(&request, &mac, "run_schedule", json!({ "id": 1 })).await;
        assert_eq!(missing, Err("m8r has nothing with that id.".to_string()));

        // Configurations: the one from before is active.
        let asked = json!({ "name": "Evenings", "prompt": "Report." });
        let evenings = call(&request, &mac, "add_configuration", asked)
            .await
            .unwrap();
        assert_eq!(
            (&evenings["id"], &evenings["active"]),
            (&json!(2), &json!(false))
        );
        let broken = json!({ "id": 2, "prompt": "{{ nope }}" });
        let refused = call(&request, &mac, "update_configuration", broken).await;
        assert!(refused
            .unwrap_err()
            .starts_with("The configuration's prompt doesn't compile."));
        let renamed = json!({ "id": 2, "name": "Nights" });
        let nights = call(&request, &mac, "update_configuration", renamed)
            .await
            .unwrap();
        assert_eq!(
            (&nights["name"], &nights["prompt"]),
            (&json!("Nights"), &json!("Report."))
        );
        let active = call(&request, &mac, "activate_configuration", json!({ "id": 2 })).await;
        assert_eq!(active.unwrap()["active"], json!(true));
        let listed = call(&request, &mac, "list_configurations", json!({}))
            .await
            .unwrap();
        let state: Vec<_> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|it| {
                (
                    it["name"].as_str().unwrap(),
                    it["active"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(state, [("Nights", true), ("Test", false)]);
        let idle = call(
            &request,
            &mac,
            "deactivate_configuration",
            json!({ "id": 2 }),
        )
        .await;
        assert_eq!(idle.unwrap()["active"], json!(false));
        let removed = call(&request, &mac, "remove_configuration", json!({ "id": 2 })).await;
        assert_eq!(removed, Ok(json!({ "removed": 2 })));
    })
    .await;
}
