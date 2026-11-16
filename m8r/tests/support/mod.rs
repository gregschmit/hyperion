//! Shared by the tests: a session and tokens for request tests, and stand-ins for the tools.

use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use loco_rs::{app::AppContext, prelude::*, TestServer};
use m8r::{
    initializers::access,
    models::{categories, configurations, runs, schedules, tokens, watchlist_items},
    settings::Settings,
};
use serde_json::{json, Value};

/// The categories that every m8r starts with.
pub const ISSUE: &str = "Issue";
pub const PR: &str = "Pull/Merge Request";

/// Open an access link, so the server's later requests carry a session. The server must save
/// cookies (`RequestConfigBuilder::save_cookies`).
pub async fn sign_in(request: &TestServer, ctx: &AppContext) {
    let res = request.get(&link_path(ctx, 60, false)).await;
    assert_eq!(res.status_code(), 303);
}

/// The path and query of a new access link.
pub fn link_path(ctx: &AppContext, seconds: u64, once: bool) -> String {
    let url = Settings::from_context(ctx).unwrap().url;
    access::link(ctx, seconds, once)
        .unwrap()
        .strip_prefix(&url)
        .unwrap()
        .to_string()
}

/// A token for the API and the MCP endpoint that covers `scope`.
pub async fn token(ctx: &AppContext, name: &str, scope: &str) -> String {
    let made = tokens::Model::create(&ctx.db, name, scope).await;
    made.unwrap().unwrap()
}

/// Put what's at `url` on the watchlist, in `category`.
pub async fn watch(ctx: &AppContext, url: &str, category: &str) -> watchlist_items::Model {
    let params = watchlist_items::Params {
        url: url.to_string(),
        category: category.to_string(),
        ..watchlist_items::Params::default()
    };
    let listed = watchlist_items::Model::create(&ctx.db, &params).await;
    listed.unwrap().unwrap().0
}

/// Make a configuration with this system message, and make it active.
pub async fn configure(ctx: &AppContext, system_message: &str) -> configurations::Model {
    let params = configurations::Params {
        name: "Test".to_string(),
        system_message: system_message.to_string(),
    };
    let made = configurations::Model::create(&ctx.db, &params).await;
    let configuration = made.unwrap().unwrap().into_active_model();
    configuration.activated(&ctx.db).await.unwrap()
}

/// What a schedule needs, for a test to change: a triage in `repo`, on weekdays, with `cat`
/// for a command, which prints the prompt back.
pub fn schedule_params(repo: &Path, prompt: &str) -> schedules::Params {
    schedules::Params {
        operation: "triage".to_string(),
        repo: repo.to_string_lossy().into_owned(),
        hron: "every weekday at 06:00".to_string(),
        command: "cat".to_string(),
        prompt: prompt.to_string(),
        ..schedules::Params::default()
    }
}

pub async fn schedule(ctx: &AppContext, params: &schedules::Params) -> schedules::Model {
    let made = schedules::Model::create(&ctx.db, params).await;
    made.unwrap().unwrap().0
}

/// Queue a run of a schedule that's made for it: `operation` in `repo`, with `command`. The
/// schedule skips its form's checks, so the repo needn't be there.
pub async fn queue(
    db: &DatabaseConnection,
    operation: &str,
    repo: &str,
    command: &str,
) -> runs::Model {
    let mut schedule = schedules::ActiveModel::new();
    schedule.operation = Set(operation.to_string());
    schedule.repo = Set(repo.to_string());
    schedule.hron = Set("every day at 06:00".to_string());
    schedule.command = Set(command.to_string());
    let schedule = schedule.insert(db).await.unwrap();
    runs::Model::queue(db, &schedule).await.unwrap()
}

/// Start `run` with `prompt`, as a tick does.
pub async fn start(db: &DatabaseConnection, run: runs::Model, prompt: &str) -> runs::Model {
    let (schedule, _) = schedules::Model::find(db, run.schedule_id.unwrap())
        .await
        .unwrap();
    let run = run.into_active_model();
    run.started(db, &schedule, prompt.to_string())
        .await
        .unwrap()
}

pub async fn category(ctx: &AppContext, name: &str) -> categories::Model {
    let found = categories::Model::named(&ctx.db, name).await;
    found.unwrap().unwrap()
}

/// A directory of stand-in tools, and a shell that finds only them. So a test doesn't depend
/// on the developer's own `gh`, `glab` or login shell.
pub struct Sandbox {
    pub dir: PathBuf,
    /// Stands in for the pages: a tick looks whether a run could reach them.
    pages: TcpListener,
}

impl Sandbox {
    /// A fresh sandbox under `target/`.
    pub fn new(name: &str) -> Self {
        let dir = std::env::current_dir()
            .unwrap()
            .join("target/test-data")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join("repo")).unwrap();
        let pages = TcpListener::bind("127.0.0.1:0").unwrap();
        let sandbox = Self { dir, pages };
        // m8r starts every tool as `<shell> -lc <script> <args>`. This one is no login shell,
        // and its PATH holds the stand-ins and the system's basics.
        let bin = sandbox.dir.join("bin");
        sandbox.tool(
            "shell",
            &format!(
                "PATH=\"{}:/usr/bin:/bin\"\nexport PATH\n[ \"$1\" = -lc ] && shift\n\
                 exec /bin/sh -c \"$@\"",
                bin.display()
            ),
        );
        sandbox
    }

    /// Put a stand-in tool on the sandbox's PATH: a shell script.
    pub fn tool(&self, name: &str, script: &str) {
        let path = self.dir.join("bin").join(name);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    pub fn remove_tool(&self, name: &str) {
        fs::remove_file(self.dir.join("bin").join(name)).unwrap();
    }

    pub fn shell(&self) -> String {
        self.dir.join("bin/shell").to_string_lossy().into_owned()
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// A clone for a schedule to run in: an empty directory.
    pub fn repo(&self) -> PathBuf {
        self.dir.join("repo")
    }

    /// Serve the app itself where the stand-in for the pages listens, so that a run's agent
    /// can use the API.
    pub fn serve(&self, router: axum::Router) {
        let listener = self.pages.try_clone().unwrap();
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await });
    }

    /// A port on this host where nothing listens.
    pub fn closed_port(&self) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    /// The app's context, with settings that use this sandbox and the agent command `agent`.
    pub fn context(&self, ctx: &AppContext, agent: &str) -> AppContext {
        let mut ctx = ctx.clone();
        ctx.config.settings = Some(self.settings(agent));
        ctx.config.server.binding = "127.0.0.1".to_string();
        ctx.config.server.port = self.pages.local_addr().unwrap().port().into();
        ctx
    }

    fn settings(&self, agent: &str) -> Value {
        json!({
            "url": "http://localhost:5150",
            "shell": self.shell(),
            "agent": agent,
            "run_limit": 1,
            "data": self.path("data"),
        })
    }
}

/// A `gh` that knows three issues: 1 is open, 2 is closed, and 3 doesn't exist. While the file
/// `logged-out` is beside it, it refuses like a `gh` with no login.
pub const GH: &str = r#"
if [ -e "$(dirname "$0")/logged-out" ]; then
  echo "gh: To get started with GitHub CLI, please run: gh auth login" >&2
  exit 4
fi
case "$4" in
  repos/o/r/issues/1) echo '{"number": 1, "title": "An open one", "state": "open"}' ;;
  repos/o/r/issues/2) echo '{"number": 2, "title": "A closed one", "state": "closed"}' ;;
  *) echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
esac
"#;

/// A `glab` that knows one merged MR and one open issue on any host.
pub const GLAB: &str = r#"
case "$4" in
  projects/g%2Fs%2Fp/merge_requests/7) echo '{"iid": 7, "title": "A merged one", "state": "merged"}' ;;
  projects/g%2Fs%2Fp/issues/8) echo '{"iid": 8, "title": "An opened one", "state": "opened"}' ;;
  *) echo '{"message": "404 Not Found"}' >&2; exit 1 ;;
esac
"#;

/// Make `path` a file with `text` in it, and the directories above it.
pub fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Run a request test with a session: the server keeps the cookie that the access link gave.
pub async fn signed_in<F, Fut>(callback: F)
where
    F: FnOnce(TestServer, AppContext) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<m8r::app::App, _, _>(config, |request, ctx| async move {
        sign_in(&request, &ctx).await;
        callback(request, ctx).await;
    })
    .await;
}

/// Where a redirect goes.
pub fn location(response: &axum_test::TestResponse) -> String {
    response.header("location").to_str().unwrap().to_string()
}
