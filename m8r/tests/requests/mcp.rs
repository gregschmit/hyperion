//! The MCP endpoint, as a client such as Claude Duet sees it.

use loco_rs::{testing::prelude::*, TestServer};
use m8r::app::App;
use serde_json::{json, Value};
use serial_test::serial;

use crate::support::{self, token, watch, ISSUES, REQUESTS};

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
        let brief = token(&ctx, "brief", "runs,repos").await;
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
                "list_repos",
                "add_repo",
                "update_repo",
                "remove_repo",
                "list_runs",
                "get_run",
            ]
        );
        let all = tools(&request, &mac).await;
        assert_eq!(all.len(), 15);

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

        // An item's URL says the rest, so it's all that adding one needs. A list can be a
        // category's or a repo's.
        let add = &for_duet[1]["inputSchema"];
        assert_eq!(add["required"], json!(["url"]));
        assert_eq!(add["additionalProperties"], json!(false));
        assert_eq!(add["properties"].get("category"), None);
        assert_eq!(add["properties"]["requested"]["type"], json!("boolean"));
        assert_eq!(
            add["properties"]["priority"]["enum"],
            json!(["medium", "high"])
        );
        let list = &for_duet[0]["inputSchema"]["properties"];
        assert_eq!(list["category"]["enum"], json!([ISSUES, REQUESTS]));
        assert_eq!(list["repo_id"]["type"], json!("integer"));
        // A schedule has a name, repos and times, and takes all the items, a category's, or
        // none.
        let schedule = all.iter().find(|tool| tool["name"] == "add_schedule");
        let schedule = &schedule.unwrap()["inputSchema"];
        assert_eq!(
            schedule["required"],
            json!(["name", "repos", "hron", "command"])
        );
        assert_eq!(
            schedule["properties"]["category"]["enum"],
            json!(["", "all", ISSUES, REQUESTS])
        );
        assert_eq!(
            schedule["properties"]["max_items"]["type"],
            json!("integer")
        );
        assert_eq!(schedule["properties"]["repos"]["type"], json!("array"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn duet_keeps_the_watchlist_with_its_tools() {
    request::<App, _, _>(|request, ctx| async move {
        let duet = token(&ctx, "duet", "watchlist_items").await;
        watch(&ctx, "https://github.com/o/r/issues/1").await;
        let repo = support::repo(&ctx.db, "~/git/g/p", "https://git.example.com/g/p").await;
        let mr = "https://git.example.com/g/p/-/merge_requests/9";

        // Someone asks for a review in a chat. The URL says the repo and the category.
        let asked = json!({ "url": mr, "requested": true, "note": "Bob asked in chat" });
        let added = call(&request, &duet, "add_watchlist_item", asked)
            .await
            .unwrap();
        assert_eq!(
            added,
            json!({
                "id": 2,
                "url": mr,
                "repo_id": repo.id,
                "repo": "~/git/g/p",
                "category": REQUESTS,
                "priority": "medium",
                "note": "Bob asked in chat",
                "requested": true,
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
                &changed["requested"]
            ),
            (&json!("high"), &json!("Bob asked in chat"), &json!(true))
        );
        // Someone asks for the issue too.
        let asked = json!({ "id": 1, "requested": true });
        let issue = call(&request, &duet, "update_watchlist_item", asked)
            .await
            .unwrap();
        assert_eq!(
            (&issue["requested"], &issue["category"]),
            (&json!(true), &json!(ISSUES))
        );

        let listed = call(&request, &duet, "list_watchlist_items", json!({}))
            .await
            .unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 2);
        // A category's items, or a repo's.
        for filter in [
            json!({ "category": REQUESTS }),
            json!({ "repo_id": repo.id }),
        ] {
            let listed = call(&request, &duet, "list_watchlist_items", filter.clone()).await;
            assert_eq!(listed, Ok(json!([changed])), "{filter}");
        }

        // What the model got wrong, it can read and put right.
        for (name, arguments, why) in [
            (
                "list_watchlist_items",
                json!({ "category": "Epic" }),
                "There's no category \"Epic\". The categories are: issues, pull/merge requests.",
            ),
            (
                "add_watchlist_item",
                json!({ "url": "https://example.com/" }),
                "That isn't the URL of an issue or a PR/MR on GitHub or GitLab.",
            ),
            (
                "add_watchlist_item",
                json!({ "url": "https://git.example.com/g/other/-/issues/1" }),
                "m8r has no repo for https://git.example.com/g/other. Add that repo first: an \
                 item is in one.",
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
        for name in ["list_schedules", "add_repo", "no_such_tool"] {
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

        // Repos: a clone that's there, and its project.
        let (src, tests) = (format!("{here}/src"), format!("{here}/tests"));
        let asked = json!({ "path": src, "url": "git@git.example.com:g/p.git" });
        let repo = call(&request, &mac, "add_repo", asked).await;
        let added = json!({ "id": 1, "path": src, "url": "https://git.example.com/g/p" });
        assert_eq!(repo, Ok(added.clone()));
        let again = call(&request, &mac, "add_repo", json!({ "path": src })).await;
        assert_eq!(again, Err(format!("m8r already has that clone: {src}.")));
        call(&request, &mac, "add_repo", json!({ "path": tests }))
            .await
            .unwrap();
        let moved = json!({ "id": 2, "url": "https://github.com/o/r" });
        let changed = call(&request, &mac, "update_repo", moved).await.unwrap();
        assert_eq!(
            (&changed["path"], &changed["url"]),
            (&json!(tests), &json!("https://github.com/o/r"))
        );
        let listed = call(&request, &mac, "list_repos", json!({})).await;
        assert_eq!(listed, Ok(json!([added, changed])));

        // A schedule that runs in the first repo, so that repo stays.
        let asked = json!({ "name": "release notes", "category": "all", "max_items": 20,
            "repos": [src], "hron": ["every 12 hours", "every day at 09:00, 17:00"],
            "command": "make release-notes" });
        let schedule = call(&request, &mac, "add_schedule", asked).await.unwrap();
        assert_eq!(
            (
                &schedule["id"],
                &schedule["name"],
                &schedule["hron"],
                &schedule["category"],
                &schedule["max_items"],
                &schedule["prompt"]
            ),
            (
                &json!(1),
                &json!("release notes"),
                &json!([
                    "every 12 hours from 00:00 to 23:59",
                    "every day at 09:00, 17:00"
                ]),
                &json!("all"),
                &json!(20),
                &json!("")
            )
        );
        assert_eq!(schedule["repos"], json!([src]));
        let in_use = call(&request, &mac, "remove_repo", json!({ "id": 1 })).await;
        assert!(in_use
            .unwrap_err()
            .contains("by 0 watchlist items and 1 schedules"));
        let gone = call(&request, &mac, "remove_repo", json!({ "id": 2 })).await;
        assert_eq!(gone, Ok(json!({ "removed": 2 })));
        let unknown = json!({ "id": 1, "repos": [tests] });
        let refused = call(&request, &mac, "update_schedule", unknown).await;
        assert_eq!(
            refused,
            Err(format!(
                "\"{tests}\" isn't one of m8r's repos. Add it to the repos first."
            ))
        );
        let every = json!({ "id": 1, "category": "", "prompt": "{{ watchlist }}" });
        let changed = call(&request, &mac, "update_schedule", every)
            .await
            .unwrap();
        assert_eq!(
            (
                &changed["category"],
                &changed["prompt"],
                &changed["command"],
                &changed["max_items"]
            ),
            (
                &json!(""),
                &json!("{{ watchlist }}"),
                &json!("make release-notes"),
                &json!(20)
            )
        );
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
                &run["name"],
                &run["repo"]
            ),
            (
                &json!(1),
                &json!("queued"),
                &json!(1),
                &json!("release notes"),
                &json!(src)
            )
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
    })
    .await;
}
