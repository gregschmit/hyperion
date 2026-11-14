use std::{fs, os::unix::fs::symlink};

use loco_rs::testing::prelude::*;
use m8r::{
    app::App,
    models::{artifacts, configurations::Operation, runs},
};
use serial_test::serial;

use crate::support::{write, Sandbox};

#[tokio::test]
#[serial]
async fn a_directory_is_an_artifact_and_a_lone_link_is_a_link() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = runs::Model::queue(db, Operation::Review, "/a")
        .await
        .unwrap()
        .unwrap();
    let dir = Sandbox::new("artifacts-collect").path("artifacts");

    write(
        &dir.join("comment on rxg!1/link.txt"),
        "  https://git.example.com/g/p/-/merge_requests/1#note_5\n",
    );
    write(&dir.join("review/index.html"), "<h1>Review</h1>");
    write(&dir.join("review/img/shot.png"), "png");
    // A link beside other files is one of the files.
    write(&dir.join("two files/link.txt"), "https://example.com/");
    write(&dir.join("two files/notes.md"), "notes");
    // Only web links count.
    write(&dir.join("script/link.txt"), "javascript:alert(1)");
    write(
        &dir.join("sentence/link.txt"),
        "see https://example.com/ for more",
    );
    // An empty directory is nothing, and so is a hidden one.
    fs::create_dir_all(dir.join("empty")).unwrap();
    write(&dir.join(".cache/file"), "x");

    let artifacts = artifacts::Model::collect(db, run.id, &dir).await.unwrap();

    let found: Vec<_> = artifacts
        .iter()
        .map(|artifact| (artifact.name.as_str(), artifact.url.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            (
                "comment on rxg!1",
                "https://git.example.com/g/p/-/merge_requests/1#note_5"
            ),
            ("review", ""),
            ("script", ""),
            ("sentence", ""),
            ("two files", ""),
        ]
    );
    assert!(!artifacts[0].hosted());
    assert!(artifacts[1].hosted());
    assert_eq!(artifacts[1].dir(&dir), dir.join("review"));

    // A second look adds nothing.
    let again = artifacts::Model::collect(db, run.id, &dir).await.unwrap();
    assert_eq!(again.len(), 5);
}

#[tokio::test]
#[serial]
async fn loose_files_become_one_artifact() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = runs::Model::queue(db, Operation::Triage, "/a")
        .await
        .unwrap()
        .unwrap();
    let dir = Sandbox::new("artifacts-loose").path("artifacts");
    write(&dir.join("TRIAGE.md"), "# Triage");
    write(&dir.join("table.csv"), "a,b");
    write(
        &dir.join("report/index.html"),
        "the agent's own report directory",
    );

    let artifacts = artifacts::Model::collect(db, run.id, &dir).await.unwrap();

    let names: Vec<_> = artifacts
        .iter()
        .map(|artifact| artifact.name.as_str())
        .collect();
    assert_eq!(names, ["report", "report-2"]);
    assert!(dir.join("report-2/TRIAGE.md").is_file());
    assert!(dir.join("report-2/table.csv").is_file());
    assert!(!dir.join("TRIAGE.md").exists());
}

#[tokio::test]
#[serial]
async fn a_lone_loose_link_is_a_link() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = runs::Model::queue(db, Operation::Triage, "/a")
        .await
        .unwrap()
        .unwrap();
    let dir = Sandbox::new("artifacts-loose-link").path("artifacts");
    write(
        &dir.join("comment.txt"),
        "https://github.com/o/r/issues/1#issuecomment-2\n",
    );

    let artifacts = artifacts::Model::collect(db, run.id, &dir).await.unwrap();

    assert_eq!(artifacts.len(), 1);
    assert_eq!(
        artifacts[0].url,
        "https://github.com/o/r/issues/1#issuecomment-2"
    );
}

#[tokio::test]
#[serial]
async fn a_run_that_left_nothing_has_no_artifacts() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let run = runs::Model::queue(db, Operation::Triage, "/a")
        .await
        .unwrap()
        .unwrap();
    let sandbox = Sandbox::new("artifacts-none");

    // No directory at all, then an empty one, then one that is only a link to elsewhere.
    assert!(
        artifacts::Model::collect(db, run.id, &sandbox.path("never-made"))
            .await
            .unwrap()
            .is_empty()
    );
    let dir = sandbox.path("artifacts");
    fs::create_dir_all(&dir).unwrap();
    assert!(artifacts::Model::collect(db, run.id, &dir)
        .await
        .unwrap()
        .is_empty());
    symlink("/etc", dir.join("etc")).unwrap();
    assert!(artifacts::Model::collect(db, run.id, &dir)
        .await
        .unwrap()
        .is_empty());
}
