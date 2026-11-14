//! Every request needs a session, and the only way to get one is an access link.
//!
//! A link carries a JWT that says when it stops working. Opening it starts a session (Loco's
//! signed cookie, which the server stops accepting after `auth.session.max_age`), and
//! redirects to the same page without the token.

use async_trait::async_trait;
use axum::{
    extract::{Request, State},
    http::{header, request::Parts, Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    Router as AxumRouter,
};
use chrono::{DateTime, Utc};
use loco_rs::prelude::Session;
use loco_rs::{
    app::{AppContext, Initializer},
    auth::jwt::JWT,
    hash, Error, Result,
};
use serde_json::{json, Map, Value};

use crate::{models::used_links, settings::Settings};

/// The query parameter that carries an access link's token.
const TOKEN: &str = "token";

/// What a session's user is called. m8r has one user: whoever holds a link.
const USER: &str = "m8r";

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

/// Make an access link that works for `seconds`, any number of times or just `once`.
///
/// # Errors
/// When the JWT settings are missing, or the secret isn't base64.
pub fn link(ctx: &AppContext, seconds: u64, once: bool) -> Result<String> {
    let expires = Utc::now().timestamp() + i64::try_from(seconds).unwrap_or(i64::MAX);
    let mut claims = Map::new();
    claims.insert("use".to_string(), json!("link"));
    claims.insert("once".to_string(), json!(once));
    claims.insert("expires".to_string(), json!(expires));
    let token = JWT::new(&ctx.config.get_jwt_config()?.secret)
        .generate_token(seconds, hash::random_string(20), claims)
        .map_err(|err| Error::string(&err.to_string()))?;
    let url = Settings::from_context(ctx)?.url;
    Ok(format!("{}/?{TOKEN}={token}", url.trim_end_matches('/')))
}

async fn guard(State(ctx): State<AppContext>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(session) = Session::from_parts(&parts, &ctx).await else {
        return refuse("m8r couldn't read your session.");
    };

    if parts.method == Method::GET {
        if let Some(token) = query_token(&parts) {
            return match redeem(&ctx, &token).await {
                Ok(()) => {
                    let session = session.renew().set_user_id(USER);
                    (session, no_store(Redirect::to(&without_token(&parts)))).into_response()
                }
                Err(why) => refuse(&why),
            };
        }
    }

    if session.user_id().is_none() {
        return refuse("You need an access link to get in.");
    }
    next.run(Request::from_parts(parts, body)).await
}

/// Check an access link's token. A single-use token is used up here.
async fn redeem(ctx: &AppContext, token: &str) -> std::result::Result<(), String> {
    let expired = "That access link has expired, or was never good.";
    let secret = &ctx.config.get_jwt_config().map_err(|_| expired)?.secret;
    let claims = JWT::new(secret)
        .validate(token)
        .map_err(|_| expired)?
        .claims;
    if claims.claims.get("use").and_then(Value::as_str) != Some("link") {
        return Err(expired.to_string());
    }
    if claims.claims.get("once").and_then(Value::as_bool) == Some(true) {
        let expires = claims
            .claims
            .get("expires")
            .and_then(Value::as_i64)
            .and_then(|at| DateTime::<Utc>::from_timestamp(at, 0))
            .ok_or(expired)?;
        let fresh = used_links::Model::spend(&ctx.db, &claims.pid, expires)
            .await
            .map_err(|_| "m8r couldn't check that access link.")?;
        if !fresh {
            return Err("That access link works one time, and it was used.".to_string());
        }
    }
    Ok(())
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

/// The page for a request with no session. It stands alone: the stylesheet needs a session
/// too.
fn refuse(why: &str) -> Response {
    let page = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"color-scheme\" content=\"light dark\"><title>m8r</title></head>\
         <body style=\"font-family: system-ui, sans-serif; max-width: 34rem; margin: 4rem auto; \
         padding: 0 1rem; line-height: 1.5\"><h1>m8r</h1><p>{why}</p>\
         <p>Make a link on the server, and open it here:</p><pre>m8r task link</pre>\
         </body></html>\n"
    );
    no_store((StatusCode::UNAUTHORIZED, Html(page))).into_response()
}
