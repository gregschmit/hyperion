use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use loco_rs::{app::AppContext, boot::run_task, prelude::*, task};
use m8r::{
    app::App,
    models::{runs::Status, schedules},
    tasks::output::follow,
};
use serial_test::serial;

use crate::support::{queue, queue_one, schedule, schedule_params, start, Sandbox};

async fn task(ctx: &AppContext, name: &str, args: &[(&str, &str)]) -> loco_rs::Result<()> {
    let vars = task::Vars::from_cli_args(
        args.iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
    );
    run_task::<App>(ctx, Some(&name.to_string()), &vars).await
}

/// What `follow` wrote, for a test to read while `follow` goes on.
#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl Written {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A command that prints, waits for the file `go` in its clone, and prints again, on stderr.
/// It gives up after a while.
const SLOW: &str = r#"
echo "first"
tries=0
until [ -e go ]; do
  tries=$((tries + 1))
  [ "$tries" -gt 300 ] && exit 1
  sleep 0.1
done
echo "second" >&2
"#;

#[tokio::test]
#[serial]
async fn what_a_run_prints_can_be_read_while_the_run_is_in_progress() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("output-follow");
    sandbox.tool("slow", SLOW);
    let ctx = sandbox.context(&boot.app_context, "slow");
    let params = schedules::Params {
        command: "slow".to_string(),
        ..schedule_params(&sandbox.repo(), "")
    };
    let run = queue_one(&ctx.db, &schedule(&ctx, &params).await).await;

    // The run waits, a tick starts it, and `follow` writes what it prints as it comes.
    let written = Written::default();
    let mut out = written.clone();
    // The command goes on only once its first line has come through, or after a while.
    let release = async {
        let mut came = false;
        for _ in 0..200 {
            came = written.text() == "first\n";
            if came {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        std::fs::write(sandbox.repo().join("go"), "").unwrap();
        came
    };
    let (ticked, followed, came) = tokio::join!(
        task(&ctx, "tick", &[]),
        follow(&ctx, run.id, &mut out),
        release
    );

    ticked.unwrap();
    assert!(came, "nothing came while the run was in progress");
    let over = followed.unwrap();
    assert_eq!(over.status, "done", "{}", over.error);
    // All of it, stdout and stderr together: what the run's row keeps.
    assert_eq!(written.text(), "first\nsecond\n");
    assert_eq!(over.output, "first\nsecond\n");
}

#[tokio::test]
#[serial]
async fn the_output_of_a_run_that_is_over_comes_from_its_row() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("output-over");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let db = &ctx.db;
    let run = start(db, queue(db, "triage", "/a", "cat").await, "p").await;

    // The run is in progress, and its file is gone already: `follow` waits for the run's end.
    let mut written = Vec::new();
    let end = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let run = run.clone().into_active_model();
        run.finished(db, Status::Failed, "printed".into(), "it broke".into())
            .await
            .unwrap()
    };
    let (followed, _) = tokio::join!(follow(&ctx, run.id, &mut written), end);
    let over = followed.unwrap();
    assert_eq!(
        (over.status.as_str(), over.error.as_str()),
        ("failed", "it broke")
    );
    assert_eq!(written, b"printed");

    // The task wants the id of a run that's there.
    assert!(task(&ctx, "output", &[]).await.is_err());
    assert!(task(&ctx, "output", &[("id", "soon")]).await.is_err());
    let unknown = task(&ctx, "output", &[("id", "99")]).await.unwrap_err();
    assert!(
        unknown.to_string().contains("there's no run 99"),
        "{unknown}"
    );
    assert!(task(&ctx, "output", &[("id", "1")]).await.is_ok());
}
