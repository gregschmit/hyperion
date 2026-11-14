#![allow(clippy::missing_errors_doc)]
use std::{
    fs,
    path::{Path as FilePath, PathBuf},
};

use axum::http::header;
use loco_rs::prelude::*;
use serde_json::json;

use crate::{models::artifacts, settings::Settings, views};

/// An agent wrote these files, maybe after it read a hostile issue, and they open inside the
/// viewer's session. So they get styles and images, and nothing else: no scripts, no forms, no
/// frames, and nothing from other sites.
const INERT: &str = "default-src 'none'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
                     font-src 'self' data:; media-src 'self'; form-action 'none'; \
                     base-uri 'none'; frame-ancestors 'none'";

/// How many files the list shows.
const FILES: usize = 500;

/// A hosted artifact: its `index.html` if it has one, else its only file, else a list of its
/// files. A link artifact goes to its link.
#[debug_handler]
pub async fn show(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<i64>,
) -> Result<Response> {
    let artifact = artifacts::Model::find(&ctx.db, id).await?;
    if !artifact.hosted() {
        return format::redirect(&artifact.url);
    }
    let Some(dir) = dir(&ctx, &artifact) else {
        return not_found();
    };
    let mut files = Vec::new();
    list(&dir, &dir, &mut files);
    files.sort();
    let open = if files.iter().any(|file| file == "index.html") {
        Some("index.html")
    } else if let [only] = &files[..] {
        Some(only.as_str())
    } else {
        None
    };
    if let Some(file) = open {
        return format::redirect(&format!("/artifacts/{id}/{}", encode(file)));
    }
    let files: Vec<_> = files
        .iter()
        .map(|file| json!({ "name": file, "href": format!("/artifacts/{id}/{}", encode(file)) }))
        .collect();
    let data = json!({ "name": artifact.name, "run_id": artifact.run_id, "files": files });
    views::page(&v, &ctx, "artifacts/show.html", "report", data).await
}

/// One file of a hosted artifact.
#[debug_handler]
pub async fn file(
    State(ctx): State<AppContext>,
    Path((id, path)): Path<(i64, String)>,
) -> Result<Response> {
    let artifact = artifacts::Model::find(&ctx.db, id).await?;
    let Some(dir) = dir(&ctx, &artifact).filter(|_| artifact.hosted()) else {
        return not_found();
    };
    // Only what's inside the artifact's directory, wherever a link in it points.
    let Some(file) = dir
        .join(&path)
        .canonicalize()
        .ok()
        .filter(|file| file.starts_with(&dir) && file.is_file())
    else {
        return not_found();
    };
    let body = fs::read(&file)?;
    let (kind, download) = content_type(&file, &body);
    let mut response = format::render()
        .header(header::CONTENT_TYPE, kind)
        .header(header::CONTENT_SECURITY_POLICY, INERT)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer");
    if download {
        response = response.header(header::CONTENT_DISPOSITION, "attachment");
    }
    Ok(response.response().body(body.into())?)
}

/// The artifact's own directory, if it's there.
fn dir(ctx: &AppContext, artifact: &artifacts::Model) -> Option<PathBuf> {
    let settings = Settings::from_context(ctx).ok()?;
    artifact
        .dir(&settings.artifacts_dir(artifact.run_id))
        .canonicalize()
        .ok()
        .filter(|dir| dir.is_dir())
}

/// The files under `dir`, as paths from `root`. Links aren't followed.
fn list(root: &FilePath, dir: &FilePath, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if files.len() >= FILES {
            return;
        }
        if kind.is_dir() {
            list(root, &entry.path(), files);
        } else if kind.is_file() {
            if let Ok(path) = entry.path().strip_prefix(root) {
                files.push(path.to_string_lossy().into_owned());
            }
        }
    }
}

/// The type to serve a file as, and whether the browser should save it, not show it. Pages,
/// styles, images and the like keep their types. Any other text shows as plain text.
fn content_type(file: &FilePath, body: &[u8]) -> (&'static str, bool) {
    let extension = file
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let known = match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "",
    };
    if !known.is_empty() {
        (known, false)
    } else if std::str::from_utf8(body).is_ok() {
        ("text/plain; charset=utf-8", false)
    } else {
        ("application/octet-stream", true)
    }
}

/// Percent-encode a file's path for a URL, and keep its slashes.
fn encode(path: &str) -> String {
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/artifacts")
        .add("/{id}", get(show))
        .add("/{id}/{*path}", get(file))
}
