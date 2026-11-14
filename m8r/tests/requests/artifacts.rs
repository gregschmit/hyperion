use std::{fs, os::unix::fs::symlink, path::PathBuf};

use loco_rs::prelude::*;
use m8r::{
    models::{artifacts, configurations::Operation, runs},
    settings::Settings,
};
use serial_test::serial;

use crate::support::{location, signed_in, write};

/// A finished run's artifacts, with their files in the test data directory.
async fn reported(ctx: &AppContext) -> PathBuf {
    let run = runs::Model::queue(&ctx.db, Operation::Review, "/a")
        .await
        .unwrap()
        .unwrap();
    let dir = Settings::from_context(ctx).unwrap().artifacts_dir(run.id);
    let _ = fs::remove_dir_all(&dir);
    write(
        &dir.join("page/index.html"),
        "<h1>Review</h1><script>alert(1)</script>",
    );
    write(
        &dir.join("page/img/a b.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    );
    write(&dir.join("notes/notes.md"), "# Notes");
    write(&dir.join("many/one.txt"), "1");
    write(&dir.join("many/two.bin"), "\u{0}");
    fs::write(dir.join("many/two.bin"), [0xff, 0xfe, 0x00]).unwrap();
    write(
        &dir.join("comment/link.txt"),
        "https://github.com/o/r/issues/1#issuecomment-2",
    );
    artifacts::Model::collect(&ctx.db, run.id, &dir)
        .await
        .unwrap();
    dir
}

#[tokio::test]
#[serial]
async fn a_hosted_artifact_is_served_inert() {
    signed_in(|request, ctx| async move {
        reported(&ctx).await;
        // In name order: comment 1, many 2, notes 3, page 4.

        // An index opens by itself.
        let res = request.get("/artifacts/4").await;
        assert_eq!(location(&res), "/artifacts/4/index.html");
        let res = request.get("/artifacts/4/index.html").await;
        assert_eq!(res.status_code(), 200);
        assert_eq!(res.header("content-type"), "text/html; charset=utf-8");
        let policy = res
            .header("content-security-policy")
            .to_str()
            .unwrap()
            .to_string();
        assert!(policy.starts_with("default-src 'none';"), "{policy}");
        assert!(policy.contains("form-action 'none'"), "{policy}");
        assert!(!policy.contains("script-src"), "{policy}");
        assert_eq!(res.header("x-content-type-options"), "nosniff");

        // Its other files come with it, and their names may need encoding.
        let res = request.get("/artifacts/4/img/a%20b.svg").await;
        assert_eq!(res.status_code(), 200);
        assert_eq!(res.header("content-type"), "image/svg+xml");
        assert!(res.header("content-security-policy").to_str().is_ok());

        // A lone file opens by itself too, and markdown is plain text.
        let res = request.get("/artifacts/3").await;
        assert_eq!(location(&res), "/artifacts/3/notes.md");
        let res = request.get("/artifacts/3/notes.md").await;
        assert_eq!(res.header("content-type"), "text/plain; charset=utf-8");
        assert_eq!(res.text(), "# Notes");

        // Several files with no index: a list. What isn't text is a download.
        let page = request.get("/artifacts/2").await.text();
        assert!(page.contains(r#"href="/artifacts/2/one.txt""#), "{page}");
        assert!(page.contains(r#"href="/artifacts/2/two.bin""#), "{page}");
        let res = request.get("/artifacts/2/two.bin").await;
        assert_eq!(res.header("content-type"), "application/octet-stream");
        assert_eq!(res.header("content-disposition"), "attachment");

        // A link artifact goes to its link, and has no files.
        let res = request.get("/artifacts/1").await;
        assert_eq!(
            location(&res),
            "https://github.com/o/r/issues/1#issuecomment-2"
        );
        assert_eq!(
            request.get("/artifacts/1/link.txt").await.status_code(),
            404
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn only_what_is_inside_the_artifact_is_served() {
    signed_in(|request, ctx| async move {
        let dir = reported(&ctx).await;
        symlink("/etc/hosts", dir.join("page/hosts")).unwrap();
        symlink(dir.join("notes"), dir.join("page/notes")).unwrap();

        for path in [
            "/artifacts/4/hosts",
            "/artifacts/4/notes/notes.md",
            "/artifacts/4/..%2Fnotes%2Fnotes.md",
            "/artifacts/4/img",
            "/artifacts/4/nope.html",
            "/artifacts/99",
            "/artifacts/99/index.html",
        ] {
            assert_eq!(request.get(path).await.status_code(), 404, "{path}");
        }
        // The router itself refuses a path that climbs.
        let res = request.get("/artifacts/4/%2e%2e/notes/notes.md").await;
        assert!(res.status_code().is_client_error());
        assert_eq!(
            request.get("/artifacts/4/index.html").await.status_code(),
            200
        );
    })
    .await;
}
