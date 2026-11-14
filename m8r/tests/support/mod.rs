//! Shared by the tests: a session for request tests, and stand-ins for the tools.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use loco_rs::{app::AppContext, TestServer};
use m8r::{initializers::access, settings::Settings};
use serde_json::{json, Value};

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

/// A directory of stand-in tools, and a shell that finds only them. So a test doesn't depend
/// on the developer's own `gh`, `glab` or login shell.
pub struct Sandbox {
    pub dir: PathBuf,
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
        let sandbox = Self { dir };
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

    /// The app's context, with settings that use this sandbox and the agent command `agent`.
    pub fn context(&self, ctx: &AppContext, agent: &str) -> AppContext {
        let mut ctx = ctx.clone();
        ctx.config.settings = Some(self.settings(agent));
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
