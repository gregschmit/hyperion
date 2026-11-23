//! The MCP endpoint, as a client such as Claude Duet sees it.

use loco_rs::{testing::prelude::*, TestServer};
use m8r::app::App;
use serde_json::{json, Value};
use serial_test::serial;

use crate::support::{self, queue, start, token, watch, ISSUES, REQUESTS};

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
        // What m8r is, and when its tools are of use. It gives the client no work of its own:
        // a client that has nothing for the watchlist leaves the tools alone.
        let description = result["serverInfo"]["description"].as_str().unwrap();
        assert!(
            description.starts_with("The developer's maintainer automation tool. Use it to"),
            "{description}"
        );
        let instructions = result["instructions"].as_str().unwrap();
        assert!(
            instructions.starts_with("m8r is the developer's maintainer automation tool. "),
            "{instructions}"
        );
        assert!(
            instructions.contains(
                "Use these tools when you need to change that watchlist, or to see what's on \
                 it.\n\nA watchlist item is"
            ),
            "{instructions}"
        );
        assert!(!instructions.contains("Your part"), "{instructions}");

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
        let planner = token(&ctx, "planner", "schedules").await;
        let mac = token(&ctx, "mac", "all").await;
        let names = |tools: &[Value]| {
            let names = tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap().to_string());
            names.collect::<Vec<_>>()
        };

        // The watchlist is the endpoint's work.
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
        // The repos and the runs are there to read, and the schedules aren't there at all.
        assert_eq!(
            names(&tools(&request, &brief).await),
            ["list_repos", "list_runs", "get_run"]
        );
        assert!(tools(&request, &planner).await.is_empty());
        let all = tools(&request, &mac).await;
        assert_eq!(all.len(), 7);

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
            // Only the watchlist changes.
            assert!(reads || name.ends_with("_watchlist_item"), "{name}");
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
            json!(["medium", "high", "urgent"])
        );
        let list = &for_duet[0]["inputSchema"]["properties"];
        assert_eq!(list["category"]["enum"], json!([ISSUES, REQUESTS]));
        assert_eq!(list["repo_id"]["type"], json!("integer"));
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
        // It matters more, and then it can't wait.
        for priority in ["high", "urgent"] {
            let raised = json!({ "id": 2, "priority": priority });
            let changed = call(&request, &duet, "update_watchlist_item", raised).await;
            assert_eq!(changed.unwrap()["priority"], json!(priority));
        }
        let changed = call(&request, &duet, "update_watchlist_item", json!({ "id": 2 }))
            .await
            .unwrap();
        assert_eq!(
            (
                &changed["priority"],
                &changed["note"],
                &changed["requested"]
            ),
            (&json!("urgent"), &json!("Bob asked in chat"), &json!(true))
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
                "update_watchlist_item",
                json!({ "id": 2, "priority": "low" }),
                "The priority is medium, high or urgent.",
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
        for name in ["list_repos", "get_run", "no_such_tool"] {
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
async fn it_reads_the_repos_and_the_runs_and_changes_neither() {
    request::<App, _, _>(|request, ctx| async move {
        let duet = token(&ctx, "duet", "watchlist_items,repos,runs").await;
        let mac = token(&ctx, "mac", "all").await;
        assert_eq!(tools(&request, &duet).await.len(), 7);

        // The repos say what m8r works in, and so which items it takes.
        let repo = support::repo(&ctx.db, "~/git/g/p", "https://git.example.com/g/p").await;
        support::repo(&ctx.db, "~/git/local", "").await;
        let listed = call(&request, &duet, "list_repos", json!({})).await;
        assert_eq!(
            listed,
            Ok(json!([
                { "id": repo.id, "path": "~/git/g/p", "url": "https://git.example.com/g/p" },
                { "id": 2, "path": "~/git/local", "url": "" },
            ]))
        );

        // The runs: newest first, and one of them whole.
        let first = queue(&ctx.db, "review", "~/git/g/p", "claude -p").await;
        start(&ctx.db, first, "The prompt.").await;
        queue(&ctx.db, "triage", "~/git/g/p", "claude -p").await;
        let listed = call(&request, &duet, "list_runs", json!({})).await.unwrap();
        let listed = listed.as_array().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|run| (&run["id"], &run["name"], &run["status"]))
                .collect::<Vec<_>>(),
            [
                (&json!(2), &json!("triage"), &json!("queued")),
                (&json!(1), &json!("review"), &json!("running")),
            ]
        );
        assert_eq!(listed[1].get("prompt"), None);
        let latest = call(&request, &duet, "list_runs", json!({ "limit": 1 })).await;
        assert_eq!(latest.unwrap().as_array().unwrap().len(), 1);
        let whole = call(&request, &duet, "get_run", json!({ "id": 1 }))
            .await
            .unwrap();
        assert_eq!(
            (&whole["prompt"], &whole["output"], &whole["repo"]),
            (&json!("The prompt."), &json!(""), &json!("~/git/g/p"))
        );
        let missing = call(&request, &duet, "get_run", json!({ "id": 9 })).await;
        assert_eq!(missing, Err("m8r has nothing with that id.".to_string()));

        // Nothing here changes a repo, a schedule or a run: not even for a token that covers
        // them all.
        for name in [
            "add_repo",
            "update_repo",
            "remove_repo",
            "list_schedules",
            "add_schedule",
            "update_schedule",
            "remove_schedule",
            "run_schedule",
        ] {
            let params = json!({ "name": name, "arguments": { "id": 1 } });
            let answer = rpc(&request, &mac, "tools/call", params).await;
            assert_eq!(
                answer["error"]["message"],
                json!(format!("Unknown tool: {name}")),
                "{name}"
            );
        }
    })
    .await;
}
