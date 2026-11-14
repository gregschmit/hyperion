//! The tools that m8r runs from its user's environment: the agent command, `gh` and `glab`.
//!
//! Each one starts through the user's login shell, so it gets the `PATH` and the environment
//! that the developer set up there. m8r installs none of them, so any of them can be missing.

use std::{fs::File, io, path::Path, process::Stdio, time::Duration};

use tokio::{
    io::AsyncWriteExt,
    process::Command,
    signal::unix::{signal, SignalKind},
};

/// How long `gh` or `glab` gets to answer.
pub const ANSWER_LIMIT: Duration = Duration::from_secs(15);

/// What the shell says when it finds no such command.
const NOT_FOUND: i32 = 127;

/// Where production gets m8r's own secret (see `config/production.yaml`). No tool needs it, and
/// an agent that prints its environment mustn't print this.
const SECRET: &str = "M8R_SECRET";

/// What a tool printed, and how it ended.
#[derive(Debug)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// The tool isn't on the user's `PATH`.
    #[must_use]
    pub fn missing(&self) -> bool {
        self.code == Some(NOT_FOUND)
    }

    /// One line that says what went wrong: what the tool printed, on stderr if it used it.
    #[must_use]
    pub fn complaint(&self) -> String {
        let said = [&self.stderr, &self.stdout]
            .into_iter()
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .find(|text| !text.is_empty());
        match (said, self.code) {
            (Some(text), _) => text.chars().take(300).collect(),
            (None, Some(code)) => format!("exited with {code}"),
            (None, None) => "was killed".to_string(),
        }
    }
}

/// Run a tool with its arguments, and return what it printed. The shell only looks the tool
/// up: the arguments never pass through its parser.
///
/// # Errors
/// When the shell can't start, or the tool takes longer than `limit`.
pub async fn run(shell: &str, argv: &[String], limit: Duration) -> io::Result<Output> {
    let mut command = Command::new(shell);
    command
        .args(["-lc", r#"exec "$@""#, "m8r"])
        .args(argv)
        .env_remove(SECRET)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(limit, command.output())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gave no answer in time"))??;
    Ok(Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Whether `program` is on the user's `PATH`.
///
/// # Errors
/// When the shell can't start.
pub async fn installed(shell: &str, program: &str) -> io::Result<bool> {
    let status = Command::new(shell)
        .args(["-lc", r#"command -v "$1""#, "m8r", program])
        .env_remove(SECRET)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status();
    let status = tokio::time::timeout(ANSWER_LIMIT, status)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gave no answer in time"))??;
    Ok(status.success())
}

/// How an agent's run ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// It exited by itself, with this status (none if a signal killed it).
    Exited(Option<i32>),
    /// It passed the time limit, and m8r killed it.
    TimedOut,
    /// m8r was told to stop, and took the agent with it.
    Interrupted,
}

impl End {
    /// Why the run failed, or nothing if it didn't.
    #[must_use]
    pub fn problem(&self, limit: Duration) -> Option<String> {
        match self {
            Self::Exited(Some(0)) => None,
            Self::Exited(Some(NOT_FOUND)) => {
                Some("the agent command wasn't found (exit status 127)".to_string())
            }
            Self::Exited(Some(code)) => Some(format!("the agent exited with status {code}")),
            Self::Exited(None) => Some("a signal killed the agent".to_string()),
            Self::TimedOut => Some(format!(
                "the run passed the time limit of {} minutes",
                limit.as_secs() / 60
            )),
            Self::Interrupted => Some("m8r stopped while the run was in progress".to_string()),
        }
    }
}

/// Start the agent `command` in `dir`, with `prompt` on its stdin, and wait for it to end.
/// What it prints, stdout and stderr together, goes to the file `output`.
///
/// # Errors
/// When the output file can't be made, or the shell can't start.
pub async fn agent(
    shell: &str,
    command: &str,
    dir: &Path,
    prompt: &str,
    output: &Path,
    limit: Duration,
) -> io::Result<End> {
    let stdout = File::create(output)?;
    let stderr = stdout.try_clone()?;
    let mut child = Command::new(shell)
        .args(["-lc", command])
        .current_dir(dir)
        .env_remove(SECRET)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        // A group of its own, so that the agent's whole tree of processes can be ended.
        .process_group(0)
        .spawn()?;
    let group = child.id();
    let stdin = child.stdin.take();
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;

    let end = {
        let wait = async {
            // The agent may read slowly, or not at all, so feed it while waiting for it.
            let feed = async {
                if let Some(mut stdin) = stdin {
                    let _ = stdin.write_all(prompt.as_bytes()).await;
                    let _ = stdin.shutdown().await;
                }
            };
            let ((), status) = tokio::join!(feed, child.wait());
            status
        };
        tokio::select! {
            status = wait => End::Exited(status?.code()),
            () = tokio::time::sleep(limit) => End::TimedOut,
            _ = terminate.recv() => End::Interrupted,
            _ = interrupt.recv() => End::Interrupted,
        }
    };

    if let Some(group) = group {
        if matches!(end, End::TimedOut | End::Interrupted) {
            // Ask first, then insist.
            signal_group(group, "-TERM").await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        // Also after a clean exit: nothing that the run started outlives it.
        signal_group(group, "-KILL").await;
    }
    if matches!(end, End::TimedOut | End::Interrupted) {
        let _ = child.wait().await;
    }
    Ok(end)
}

/// Send a signal to every process in the group.
async fn signal_group(group: u32, signal: &str) {
    let _ = Command::new("kill")
        .args([signal, "--", &format!("-{group}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}
