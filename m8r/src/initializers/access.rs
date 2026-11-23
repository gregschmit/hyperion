//! Nothing answers without a session, a token or a share link, and one guard decides for every
//! path: a page, the API, a static file.
//!
//! Who asks is one of three:
//!
//! - The holder of a session, who covers everything. The only way to get a session is an access
//!   link. A link carries a short signed token that says when it stops working. Opening it
//!   starts a session (Loco's signed cookie, which the server stops accepting after
//!   `auth.session.max_age`), and redirects to the same address without the token.
//! - The holder of a token, in an `Authorization: Bearer` header, who covers the token's
//!   scope. A token is one that `task token` made, or a run's own, which works while the run
//!   does.
//! - The holder of a run's share link, who reads that run until the link's token says, and
//!   nothing else.
//!
//! What they can use is the same for a page and for the API. A path is part of a resource:
//! `/schedules` and `/api/schedules` are both `schedules`. A scope names the resources that
//! it covers.
//!
//! A browser sends a session's cookie with every request to m8r, whichever site's page makes
//! the request. So on a session, a request that changes something must come from one of
//! m8r's own pages. A token is in a header, which no other site can make a browser send.
//!
//! The MCP endpoint (`/mcp`) takes a token and nothing else: its clients aren't browsers. Its
//! tools look at the token's scope themselves.

use std::time::Duration;

use async_trait::async_trait;
use axum::{
    extract::{Request, State},
    http::{header, request::Parts, Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    Json, Router as AxumRouter,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use loco_rs::prelude::Session;
use loco_rs::{
    app::{AppContext, Initializer},
    auth::jwt::JWT,
    hash, Error, Result,
};
use serde_json::{json, Map, Value};
use sha2::Sha256;

use crate::{
    models::{runs, tokens, used_links},
    settings::Settings,
};

/// The query parameter that carries a link's token: an access link's, or a share link's.
const TOKEN: &str = "token";

/// How much of an HMAC-SHA256 a link's token carries. Half of it is plenty to keep anyone
/// without the secret from making one, and it keeps a link short.
const SEAL: usize = 16;

/// How long a run's share link works.
pub const SHARE_LIMIT: Duration = Duration::from_secs(72 * 60 * 60);

/// What a session's user is called. m8r has one user: whoever holds a link.
const USER: &str = "m8r";

/// What the holder of a session is called, where a token's holder is called by the token's
/// name.
const SESSION: &str = "session";

/// What a run's token covers: the watchlist, which m8r's part of the prompt tells the agent
/// to keep.
const RUN_SCOPE: &str = "watchlist_items";

/// Who makes a request. The guard puts it in the request, for what answers.
#[derive(Debug, Clone)]
pub enum Caller {
    /// Someone who covers a scope (see `tokens::covers`): the holder of a session, called
    /// `session`, or of a token, called by the token's name, or `run <id>` for a run's own.
    Holder { name: String, scope: String },
    /// The holder of this run's share link, who reads the run: its page shows no way to the
    /// rest of m8r.
    Guest { run_id: i64 },
}

impl Caller {
    #[must_use]
    pub fn covers(&self, resource: &str) -> bool {
        match self {
            Self::Holder { scope, .. } => tokens::covers(scope, resource),
            Self::Guest { .. } => false,
        }
    }

    #[must_use]
    pub fn is_guest(&self) -> bool {
        matches!(self, Self::Guest { .. })
    }
}

pub struct AccessInitializer;

#[async_trait]
impl Initializer for AccessInitializer {
    fn name(&self) -> String {
        "access".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, ctx: &AppContext) -> Result<AxumRouter> {
        Ok(router.layer(middleware::from_fn_with_state(ctx.clone(), guard)))
    }
}

/// What an access link's token says: whether the link works one time, when it stops working,
/// and a name of its own, by which `used_links` knows a one-time link that was used.
#[derive(Debug, PartialEq, Eq)]
struct Link {
    once: bool,
    /// In seconds since 1970.
    expires: u32,
    name: [u8; Link::NAME],
}

impl Link {
    const NAME: usize = 8;
    /// The bytes that say the three things above.
    const FACTS: usize = 1 + 4 + Self::NAME;

    /// The token: the facts and their seal, as text that fits in an address. 29 bytes, so 39
    /// characters, where a JWT of the same has some 250.
    fn token(&self, secret: &str) -> Option<String> {
        let mut bytes = vec![u8::from(self.once)];
        bytes.extend(self.expires.to_be_bytes());
        bytes.extend(self.name);
        let seal = Self::seal(secret, &bytes)?.finalize().into_bytes();
        bytes.extend(&seal[..SEAL]);
        Some(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Read a token that m8r made with this secret. Any other text is none.
    fn read(secret: &str, token: &str) -> Option<Self> {
        let bytes = URL_SAFE_NO_PAD.decode(token).ok()?;
        if bytes.len() != Self::FACTS + SEAL {
            return None;
        }
        let (facts, seal) = bytes.split_at(Self::FACTS);
        // In constant time: the answer mustn't say how much of a guess was right.
        Self::seal(secret, facts)?
            .verify_truncated_left(seal)
            .ok()?;
        Some(Self {
            once: facts[0] == 1,
            expires: u32::from_be_bytes(facts[1..5].try_into().ok()?),
            name: facts[5..].try_into().ok()?,
        })
    }

    /// The HMAC of a link's facts under m8r's secret. Its first words keep it apart from
    /// anything else that the secret signs.
    fn seal(secret: &str, facts: &[u8]) -> Option<Hmac<Sha256>> {
        let mut seal = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
        seal.update(b"m8r access link\n");
        seal.update(facts);
        Some(seal)
    }
}

/// What a share link's token says: when the link stops working, in seconds since 1970. The
/// run that it opens is in the link's address, and the seal covers both, so a token fits no
/// other run.
#[derive(Debug, PartialEq, Eq)]
struct Share {
    expires: u32,
}

impl Share {
    /// The bytes that say when.
    const FACTS: usize = 4;

    /// The token for run `run_id`: 20 bytes, so 27 characters.
    fn token(&self, secret: &str, run_id: i64) -> Option<String> {
        let mut bytes = self.expires.to_be_bytes().to_vec();
        let seal = self.seal(secret, run_id)?.finalize().into_bytes();
        bytes.extend(&seal[..SEAL]);
        Some(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Read a token that m8r made with this secret for run `run_id`. Any other text is none.
    fn read(secret: &str, run_id: i64, token: &str) -> Option<Self> {
        let bytes = URL_SAFE_NO_PAD.decode(token).ok()?;
        if bytes.len() != Self::FACTS + SEAL {
            return None;
        }
        let (expires, seal) = bytes.split_at(Self::FACTS);
        let share = Self {
            expires: u32::from_be_bytes(expires.try_into().ok()?),
        };
        // In constant time: the answer mustn't say how much of a guess was right.
        share
            .seal(secret, run_id)?
            .verify_truncated_left(seal)
            .ok()?;
        Some(share)
    }

    /// Whether the link still works.
    fn works(&self) -> bool {
        Utc::now().timestamp() <= i64::from(self.expires)
    }

    /// The HMAC of the run and the time under m8r's secret. Its first words keep it apart
    /// from an access link's.
    fn seal(&self, secret: &str, run_id: i64) -> Option<Hmac<Sha256>> {
        let mut seal = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
        seal.update(b"m8r share link\n");
        seal.update(&run_id.to_be_bytes());
        seal.update(&self.expires.to_be_bytes());
        Some(seal)
    }
}

/// The time `seconds` from now, as a link's token holds it.
fn expiry(seconds: u64) -> u32 {
    let now = u64::try_from(Utc::now().timestamp()).unwrap_or_default();
    u32::try_from(now.saturating_add(seconds)).unwrap_or(u32::MAX)
}

/// Make an access link that works for `seconds`, any number of times or just `once`.
///
/// # Errors
/// When the JWT settings, which hold m8r's secret, are missing.
pub fn link(ctx: &AppContext, seconds: u64, once: bool) -> Result<String> {
    let mut name = [0; Link::NAME];
    name.copy_from_slice(hash::random_string(Link::NAME).as_bytes());
    let link = Link {
        once,
        expires: expiry(seconds),
        name,
    };
    let token = link
        .token(&ctx.config.get_jwt_config()?.secret)
        .ok_or_else(|| Error::string("m8r couldn't sign the link"))?;
    let url = Settings::from_context(ctx)?.url;
    Ok(format!("{}/?{TOKEN}={token}", url.trim_end_matches('/')))
}

/// Make a share link for run `run_id`, and say when it stops working. For `seconds`, it opens
/// that run with no session, to read, and nothing else.
///
/// # Errors
/// When the JWT settings, which hold m8r's secret, are missing.
pub fn share_link(ctx: &AppContext, run_id: i64, seconds: u64) -> Result<(String, DateTime<Utc>)> {
    let share = Share {
        expires: expiry(seconds),
    };
    let token = share
        .token(&ctx.config.get_jwt_config()?.secret, run_id)
        .ok_or_else(|| Error::string("m8r couldn't sign the link"))?;
    let until = DateTime::<Utc>::from_timestamp(share.expires.into(), 0).unwrap_or_default();
    let url = Settings::from_context(ctx)?.url;
    let link = format!(
        "{}/runs/{run_id}?{TOKEN}={token}",
        url.trim_end_matches('/')
    );
    Ok((link, until))
}

/// Make the token that lets run `run_id` use the API. It works while the run is running, and
/// for `seconds` at most.
///
/// # Errors
/// When the JWT settings are missing, or the secret isn't base64.
pub fn run_token(ctx: &AppContext, run_id: i64, seconds: u64) -> Result<String> {
    let mut claims = Map::new();
    claims.insert("use".to_string(), json!("run"));
    claims.insert("run".to_string(), json!(run_id));
    JWT::new(&ctx.config.get_jwt_config()?.secret)
        .generate_token(seconds, hash::random_string(20), claims)
        .map_err(|err| Error::string(&err.to_string()))
}

/// How a request got in.
enum Entry {
    /// Someone m8r knows. `cookie`: a session's cookie says who, and a browser sends that by
    /// itself.
    Caller { caller: Caller, cookie: bool },
    /// An access link, which started a session. The answer sends the browser to the same
    /// address without the token.
    SignedIn(Response),
}

/// Why a request gets nothing.
struct Refusal {
    status: StatusCode,
    why: String,
    /// What to do about it, as HTML. Only a page says it.
    how: &'static str,
}

impl Refusal {
    fn new(status: StatusCode, why: impl Into<String>) -> Self {
        Self {
            status,
            why: why.into(),
            how: "",
        }
    }

    /// For someone that m8r doesn't know.
    fn unknown(why: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, why)
    }

    /// For someone with no session, or with a link that starts none: how to get a session.
    fn no_session(why: impl Into<String>) -> Self {
        Self {
            how: "<p>Make a link on the server, and open it here:</p><pre>m8r task link</pre>",
            ..Self::unknown(why)
        }
    }

    fn failed<E>(_: E) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "m8r couldn't check the token.",
        )
    }

    /// The answer: JSON in the shape of Loco's errors, or a page that stands alone, since the
    /// stylesheet needs a session too.
    fn answer(self, json: bool) -> Response {
        let Self { status, why, how } = self;
        if !json {
            // The reason can hold a token's name, which is someone's own words.
            let why = why
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let page = format!(
                "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
                 <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
                 <meta name=\"color-scheme\" content=\"light dark\"><title>m8r</title></head>\
                 <body style=\"font-family: system-ui, sans-serif; max-width: 34rem; \
                 margin: 4rem auto; padding: 0 1rem; line-height: 1.5\"><h1>m8r</h1>\
                 <p>{why}</p>{how}</body></html>\n"
            );
            return no_store((status, Html(page))).into_response();
        }
        let error = match status {
            StatusCode::UNAUTHORIZED => "unauthorized",
            StatusCode::FORBIDDEN => "forbidden",
            _ => "internal_server_error",
        };
        let body = Json(json!({ "error": error, "description": why }));
        if status == StatusCode::UNAUTHORIZED {
            return (status, [(header::WWW_AUTHENTICATE, "Bearer")], body).into_response();
        }
        (status, body).into_response()
    }
}

async fn guard(State(ctx): State<AppContext>, request: Request, next: Next) -> Response {
    let (mut parts, body) = request.into_parts();
    let path = parts.uri.path().to_string();
    let mcp = path == "/mcp" || path.starts_with("/mcp/");
    // The API and the MCP endpoint answer in JSON. Anything else is for a reader.
    let json = mcp || path == "/api" || path.starts_with("/api/");

    let entry = if mcp {
        with_token(&ctx, &parts).await
    } else {
        entry(&ctx, &parts, json).await
    };
    let (caller, cookie) = match entry {
        Ok(Entry::Caller { caller, cookie }) => (caller, cookie),
        Ok(Entry::SignedIn(redirect)) => return redirect,
        Err(refusal) => return refusal.answer(json),
    };
    if cookie && changes(&parts.method) && !from_m8r(&parts) {
        let why = "On a session, a request that changes something must come from one of m8r's \
                   own pages.";
        return Refusal::new(StatusCode::FORBIDDEN, why).answer(json);
    }
    let covered = match &caller {
        // The MCP endpoint's tools look at the scope themselves.
        Caller::Holder { .. } if mcp => true,
        Caller::Holder { scope, .. } => {
            resource(&path).is_none_or(|resource| tokens::covers(scope, resource))
        }
        // The link's token was read for the address that the request asks for.
        Caller::Guest { .. } => true,
    };
    if let (false, Caller::Holder { name, .. }) = (covered, &caller) {
        let why = format!("The token {name} doesn't cover this.");
        return Refusal::new(StatusCode::FORBIDDEN, why).answer(json);
    }

    let guest = caller.is_guest();
    parts.extensions.insert(caller);
    let answer = next.run(Request::from_parts(parts, body)).await;
    if guest {
        return for_guest(answer);
    }
    answer
}

/// The resource that a path is part of, which its caller must cover. A page and the API name
/// it the same way: the path's first part, or the one after `/api`. None: a static file,
/// which is for whoever gets in.
fn resource(path: &str) -> Option<&str> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let (first, rest) = path.split_once('/').unwrap_or((path, ""));
    match first {
        "static" => None,
        "api" => Some(rest.split('/').next().unwrap_or_default()),
        // The home page lists the runs.
        "" => Some("runs"),
        "watchlist" => Some("watchlist_items"),
        page => Some(page),
    }
}

/// The run that a path is of, if a share link opens the path: the run's page, the run in the
/// API, and what the run prints as it comes.
fn shared_run(path: &str) -> Option<i64> {
    let run = match path.strip_prefix("/api") {
        Some(api) => {
            let run = api.strip_prefix("/runs/")?;
            run.strip_suffix("/output").unwrap_or(run)
        }
        None => path.strip_prefix("/runs/")?,
    };
    run.parse().ok()
}

/// Whether a request with this method changes something.
fn changes(method: &Method) -> bool {
    ![Method::GET, Method::HEAD, Method::OPTIONS].contains(method)
}

/// Whether the request comes from one of m8r's own pages, as the browser says. A browser
/// says how the page's site compares to the request's (`Sec-Fetch-Site`). An old one, or one
/// that doesn't trust the address, says only where the page is (`Origin`): that must be
/// where the request goes (`Host`).
fn from_m8r(parts: &Parts) -> bool {
    let said = |name: &str| parts.headers.get(name).and_then(|said| said.to_str().ok());
    if let Some(site) = said("sec-fetch-site") {
        return site == "same-origin";
    }
    let (Some(origin), Some(host)) = (said("origin"), said("host")) else {
        return false;
    };
    origin
        .split_once("://")
        .is_some_and(|(_, place)| place == host)
}

/// How a request gets in to a page, the API or a static file, or why it doesn't.
async fn entry(ctx: &AppContext, parts: &Parts, json: bool) -> std::result::Result<Entry, Refusal> {
    if bearer(parts).is_some() {
        return with_token(ctx, parts).await;
    }
    let session = Session::from_parts(parts, ctx)
        .await
        .map_err(|_| Refusal::no_session("m8r couldn't read your session."))?;

    if parts.method == Method::GET {
        if let Some(token) = query_token(parts) {
            // A share link's token opens its run, session or none.
            match shared(ctx, parts, &token) {
                Some((run_id, share)) if share.works() => {
                    let caller = Caller::Guest { run_id };
                    return Ok(Entry::Caller {
                        caller,
                        cookie: false,
                    });
                }
                Some(_) => {
                    let why = "That link to a run has expired. Ask for a new one.";
                    return Err(Refusal::unknown(why));
                }
                None => {}
            }
            redeem(ctx, &token).await.map_err(Refusal::no_session)?;
            let session = session.renew().set_user_id(USER);
            let redirect = no_store(Redirect::to(&without_token(parts)));
            return Ok(Entry::SignedIn((session, redirect).into_response()));
        }
    }

    if session.user_id().is_none() {
        // A page's reader needs a link. The API's caller is more likely to have a token.
        return Err(if json {
            Refusal::unknown("You need a session, or a token: \"Authorization: Bearer <token>\".")
        } else {
            Refusal::no_session("You need an access link to get in.")
        });
    }
    let caller = Caller::Holder {
        name: SESSION.to_string(),
        scope: tokens::ALL.to_string(),
    };
    Ok(Entry::Caller {
        caller,
        cookie: true,
    })
}

/// The holder of the token in the request's header, or why the request gets nothing.
async fn with_token(ctx: &AppContext, parts: &Parts) -> std::result::Result<Entry, Refusal> {
    let Some(token) = bearer(parts) else {
        let why = "Send a token: \"Authorization: Bearer <token>\".";
        return Err(Refusal::unknown(why));
    };
    let holder = |name: String, scope: &str| {
        let caller = Caller::Holder {
            name,
            scope: scope.to_string(),
        };
        Entry::Caller {
            caller,
            cookie: false,
        }
    };

    if token.starts_with(tokens::PREFIX) {
        let row = tokens::Model::of(&ctx.db, token)
            .await
            .map_err(Refusal::failed)?
            .ok_or_else(|| Refusal::unknown("That token isn't m8r's, or it was revoked."))?;
        return Ok(holder(row.name, &row.scope));
    }

    // A run's own token: a JWT that names the run.
    let secret = &ctx.config.get_jwt_config().map_err(Refusal::failed)?.secret;
    let claims = JWT::new(secret)
        .validate(token)
        .ok()
        .map(|data| data.claims);
    let run_id = claims
        .filter(|claims| claims.claims.get("use").and_then(Value::as_str) == Some("run"))
        .and_then(|claims| claims.claims.get("run").and_then(Value::as_i64))
        .ok_or_else(|| Refusal::unknown("That token isn't m8r's, or it expired."))?;
    let running = runs::Model::is_running(&ctx.db, run_id)
        .await
        .map_err(Refusal::failed)?;
    if !running {
        return Err(Refusal::unknown("That run is over, and so is its token."));
    }
    Ok(holder(format!("run {run_id}"), RUN_SCOPE))
}

/// The token of an `Authorization: Bearer <token>` header.
fn bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then_some(token.trim())
}

/// Check an access link's token. A single-use token is used up here.
async fn redeem(ctx: &AppContext, token: &str) -> std::result::Result<(), String> {
    let expired = "That access link has expired, or was never good.";
    let secret = &ctx.config.get_jwt_config().map_err(|_| expired)?.secret;
    let link = Link::read(secret, token).ok_or(expired)?;
    let expires = DateTime::<Utc>::from_timestamp(link.expires.into(), 0).ok_or(expired)?;
    if Utc::now() > expires {
        return Err(expired.to_string());
    }
    if link.once {
        let name = String::from_utf8_lossy(&link.name);
        let fresh = used_links::Model::spend(&ctx.db, &name, expires)
            .await
            .map_err(|_| "m8r couldn't check that access link.")?;
        if !fresh {
            return Err("That access link works one time, and it was used.".to_string());
        }
    }
    Ok(())
}

/// The run that the request asks for and what its share link's token says, if the request's
/// token is that. None: it may be an access link's token.
fn shared(ctx: &AppContext, parts: &Parts, token: &str) -> Option<(i64, Share)> {
    let run_id = shared_run(parts.uri.path())?;
    let secret = &ctx.config.get_jwt_config().ok()?.secret;
    Some((run_id, Share::read(secret, run_id, token)?))
}

/// What a guest gets is kept nowhere, and a link on it tells the next site nothing of this
/// address, which holds the token.
fn for_guest(page: Response) -> Response {
    let headers = [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
    ];
    (headers, page).into_response()
}

fn query_token(parts: &Parts) -> Option<String> {
    let pair = parts
        .uri
        .query()?
        .split('&')
        .find(|pair| pair.split('=').next() == Some(TOKEN))?;
    Some(pair.split_once('=')?.1.to_string())
}

/// The request's own path and query, with the token taken out.
fn without_token(parts: &Parts) -> String {
    let query: Vec<&str> = parts
        .uri
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty() && pair.split('=').next() != Some(TOKEN))
        .collect();
    if query.is_empty() {
        parts.uri.path().to_string()
    } else {
        format!("{}?{}", parts.uri.path(), query.join("&"))
    }
}

fn no_store(response: impl IntoResponse) -> impl IntoResponse {
    ([(header::CACHE_CONTROL, "no-store")], response)
}
