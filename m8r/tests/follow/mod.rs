//! What a run prints, as a follower tells it while the file grows.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use loco_rs::prelude::*;
use m8r::{
    app::App,
    follow::{Change, Follower, Watch},
    models::runs::Status,
    settings::Settings,
};
use serial_test::serial;

use crate::{
    stream::{piece, whole, PARTS, STEPS, STREAM},
    support::{queue, start, Sandbox},
};

/// Add to the end of the file, as a run's command does when it prints.
fn print(file: &Path, more: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .unwrap();
    file.write_all(more.as_bytes()).unwrap();
}

fn raw(text: &str) -> Change {
    Change::Raw(text.to_string())
}

/// A step that the agent said: whole, or still `open`.
fn said(id: usize, source: &str, open: bool) -> Change {
    Change::Step {
        id,
        source: source.to_string(),
        tool: false,
        open,
    }
}

fn tool(id: usize, source: &str) -> Change {
    Change::Step {
        id,
        source: source.to_string(),
        tool: true,
        open: false,
    }
}

/// The text that the changes add up to.
fn text(changes: &[Change]) -> String {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Raw(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_follower_tells_a_stream_as_the_agent_writes_it() {
    let sandbox = Sandbox::new("follow-stream");
    let file = sandbox.path("run.out");
    let mut follower = Follower::new(file.clone());
    let lines: Vec<&str> = PARTS.lines().collect();
    let mut all = Vec::new();
    let mut told = |follower: &mut Follower| {
        let changes = follower.poll();
        all.extend(changes.clone());
        changes
    };

    // Nothing while the file isn't there, and nothing while its first line isn't whole: that
    // line says what the file is.
    assert_eq!(told(&mut follower), []);
    print(&file, &lines[0][..30]);
    assert_eq!(told(&mut follower), []);
    print(&file, &format!("{}\n", &lines[0][30..]));
    assert_eq!(told(&mut follower), []);

    // The first piece of what the agent says starts a step, which is open: the agent is
    // writing it. The text has the same piece.
    print(&file, &format!("{}\n", lines[1..9].join("\n")));
    assert_eq!(
        told(&mut follower),
        [raw("I'll read "), said(0, "I'll read ", true)]
    );
    // The next piece is more of both. Half a line says nothing until the other half comes.
    let (half, other) = lines[9].split_at(60);
    print(&file, half);
    assert_eq!(told(&mut follower), []);
    print(&file, &format!("{other}\n"));
    assert_eq!(
        told(&mut follower),
        [
            raw("the MR first."),
            said(0, "I'll read the MR first.", true)
        ]
    );
    // The whole of it is the text that the pieces made: only the step changes, which is
    // open no more.
    print(&file, &format!("{}\n", lines[10]));
    assert_eq!(
        told(&mut follower),
        [said(0, "I'll read the MR first.", false)]
    );
    assert_eq!(told(&mut follower), []);

    // A tool is a step, and a line of the text. Two that follow each other are one list.
    let bash = "- **Bash** ` command: glab mr view 12 --comments, description: Read MR 12 `";
    let agent = "- **Agent** ` description: Review MR 12, prompt: Review it. `";
    print(&file, &format!("{}\n", lines[11..25].join("\n")));
    assert_eq!(
        told(&mut follower),
        [
            raw(&format!("\n\n{bash}\n{agent}")),
            tool(1, bash),
            tool(2, agent)
        ]
    );

    // Several pieces at once are one change of the text, and the step as it is now.
    print(&file, &format!("{}\n", lines[25..29].join("\n")));
    assert_eq!(
        told(&mut follower),
        [
            raw("\n\n## Review\n\nLooks **good**."),
            said(3, "## Review\n\nLooks **good**.", true)
        ]
    );
    print(&file, &format!("{}\n", lines[29..].join("\n")));
    assert_eq!(
        told(&mut follower),
        [said(3, "## Review\n\nLooks **good**.", false)]
    );

    // All in all, the text is the steps as markdown.
    assert_eq!(text(&all), STEPS);

    // A follower that starts late tells the same, at once. So does one of a stream that has
    // no pieces.
    let late = Follower::new(file.clone()).poll();
    assert_eq!(text(&late), STEPS);
    assert_eq!(late.len(), 5);
    assert_eq!(late[4], said(3, "## Review\n\nLooks **good**.", false));
    fs::write(&file, STREAM).unwrap();
    let plain = Follower::new(file).poll();
    assert_eq!(plain, late);
}

#[test]
fn a_follower_tells_text_as_it_is_printed() {
    let sandbox = Sandbox::new("follow-text");
    let file = sandbox.path("run.out");
    let mut follower = Follower::new(file.clone());

    // Text is one step, which stays open: all of it, each time. The text itself comes a part
    // at a time.
    print(&file, "Work");
    assert_eq!(follower.poll(), [raw("Work"), said(0, "Work", true)]);
    assert_eq!(follower.poll(), []);
    // A character that's only half there waits for its other half.
    let (half, other) = "ing…\n".as_bytes().split_at(4);
    let mut printed = OpenOptions::new().append(true).open(&file).unwrap();
    printed.write_all(half).unwrap();
    assert_eq!(follower.poll(), [raw("ing"), said(0, "Working", true)]);
    printed.write_all(other).unwrap();
    assert_eq!(follower.poll(), [raw("…\n"), said(0, "Working…\n", true)]);

    // JSON that isn't Claude Code's stream is text too, once its first line is whole.
    let other = sandbox.path("other.out");
    let mut follower = Follower::new(other.clone());
    print(&other, "{\"type\":\"thread.started\"}");
    assert_eq!(follower.poll(), []);
    print(&other, "\n");
    let line = "{\"type\":\"thread.started\"}\n";
    assert_eq!(follower.poll(), [raw(line), said(0, line, true)]);
}

#[test]
fn a_follower_reads_a_long_file_from_its_end() {
    let sandbox = Sandbox::new("follow-long");
    let file = sandbox.path("run.out");
    // Some 6 MB of steps.
    let filler = "x".repeat(2000);
    let mut long = format!("{}\n", STREAM.lines().next().unwrap());
    for number in 0..3000 {
        long.push_str(&whole(&format!("step {number} {filler}")));
        long.push('\n');
    }
    fs::write(&file, &long).unwrap();

    let mut follower = Follower::new(file.clone());
    let changes = follower.poll();
    // It says first that steps are left out, as a step and in the text. Then the steps of
    // the file's end, from the first whole line.
    let note = "*The steps before these are left out.*";
    assert_eq!(changes[1], said(0, note, false));
    let Change::Step { id: 1, source, .. } = &changes[2] else {
        panic!("{:?}", changes[2]);
    };
    assert!(source.starts_with("step 1"), "{}", &source[..20]);
    assert!(text(&changes).starts_with(&format!("{note}\n\nstep 1")));
    let last = changes.len() - 1;
    assert!(
        matches!(&changes[last], Change::Step { source, .. } if source.starts_with("step 2999 "))
    );

    // What comes later goes on from there.
    print(&file, &format!("{}\n", piece("More")));
    assert_eq!(follower.poll(), [raw("\n\nMore"), said(last, "More", true)]);
}

#[test]
fn a_follower_tells_all_anew_when_what_it_told_no_longer_holds() {
    let sandbox = Sandbox::new("follow-anew");
    let file = sandbox.path("run.out");
    let init = STREAM.lines().next().unwrap();
    fs::write(&file, format!("{init}\n{}\n", piece("Looks good"))).unwrap();
    let mut follower = Follower::new(file.clone());
    assert_eq!(
        follower.poll(),
        [raw("Looks good"), said(0, "Looks good", true)]
    );

    // The whole of a text isn't what its pieces made. The text that was told can't be taken
    // back, so all of it starts again.
    print(&file, &format!("{}\n", whole("Looks bad.")));
    assert_eq!(
        follower.poll(),
        [
            Change::Start,
            raw("Looks bad."),
            said(0, "Looks bad.", false)
        ]
    );
    assert_eq!(follower.poll(), []);
}

#[tokio::test]
#[serial]
async fn a_watch_follows_a_run_from_its_start_until_it_is_over() {
    let boot = boot_test::<App>().await.unwrap();
    let sandbox = Sandbox::new("follow-watch");
    let ctx = sandbox.context(&boot.app_context, "cat");
    let db = &ctx.db;
    let run = queue(db, "review", "/home/m8r/git/rxg", "claude -p").await;
    let file = Settings::from_context(&ctx).unwrap().output_file(run.id);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    let lines: Vec<&str> = PARTS.lines().collect();

    // It says first that all comes anew, for a page that had something before.
    let mut watch = Watch::new(&ctx, &run).unwrap();
    assert_eq!(watch.next().await, Some(Change::Start));

    // The run waits, then starts: its page has more to say of it then.
    let run = start(db, run, "The prompt.").await;
    assert_eq!(watch.next().await, Some(Change::Started));

    // What the command prints comes as it's printed.
    print(&file, &format!("{}\n", lines[..9].join("\n")));
    assert_eq!(watch.next().await, Some(raw("I'll read ")));
    assert_eq!(watch.next().await, Some(said(0, "I'll read ", true)));
    print(&file, &format!("{}\n", lines[9]));
    assert_eq!(watch.next().await, Some(raw("the MR first.")));
    assert_eq!(
        watch.next().await,
        Some(said(0, "I'll read the MR first.", true))
    );

    // The run ends, and the tick removes the file. What the command printed last still
    // comes, and then the watch is over.
    print(&file, &format!("{}\n", lines[10]));
    fs::remove_file(&file).unwrap();
    run.into_active_model()
        .finished(db, Status::Done, "# Review".to_string(), String::new())
        .await
        .unwrap();
    assert_eq!(
        watch.next().await,
        Some(said(0, "I'll read the MR first.", false))
    );
    assert_eq!(watch.next().await, Some(Change::Over));
    assert_eq!(watch.next().await, None);

    // A watch of a run that's over says so at once.
    let run = m8r::models::runs::Model::find(db, 1).await.unwrap();
    let mut watch = Watch::new(&ctx, &run).unwrap();
    assert_eq!(watch.next().await, Some(Change::Start));
    assert_eq!(watch.next().await, Some(Change::Over));
    assert_eq!(watch.next().await, None);
}
