//! A tiny OAuth 2.1 authorization server plus a protected MCP endpoint, for tests.
//!
//! `mcp-studio-testserver --oauth <port>` serves, on one port:
//!
//! - `/mcp`: the reference MCP server over Streamable HTTP, answering 401 without a valid bearer token;
//! - the discovery documents, dynamic client registration, `/authorize` (which immediately redirects
//!   back with a code, like a user who clicked "allow"), and `/token` (authorization code and refresh
//!   token grants);
//! - `/_stats`: counters, so tests can check what happened (e.g. that a token was refreshed).
//!
//! Access tokens are short-lived (`expires_in` = 2 seconds) to exercise automatic refresh.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex,
};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Json, Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde::Deserialize;
use serde_json::json;

use crate::TestServer;

#[derive(Default)]
struct State_ {
    codes_issued: AtomicU32,
    refreshes: AtomicU32,
    unauthorized: AtomicU32,
    registrations: AtomicU32,
    /// `client_id` and `scope` of the last authorization request.
    last_authorization: Mutex<(String, String)>,
    /// Access tokens that are currently valid.
    valid_tokens: Mutex<Vec<String>>,
}

type Shared = Arc<State_>;

pub fn router(port: u16) -> Router {
    let state: Shared = Arc::new(State_::default());
    let base = format!("http://127.0.0.1:{port}");

    let mcp: StreamableHttpService<TestServer, LocalSessionManager> = StreamableHttpService::new(
        || Ok(TestServer::new()),
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let protected = Router::new()
        .nest_service("/mcp", mcp)
        .layer(middleware::from_fn_with_state(
            (state.clone(), base.clone()),
            require_token,
        ));

    let resource_metadata = {
        let base = base.clone();
        move || {
            let base = base.clone();
            async move {
                Json(json!({
                    "resource": format!("{base}/mcp"),
                    "authorization_servers": [base],
                    "bearer_methods_supported": ["header"],
                }))
            }
        }
    };
    let server_metadata = {
        let base = base.clone();
        move || {
            let base = base.clone();
            async move {
                Json(json!({
                    "issuer": base,
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "registration_endpoint": format!("{base}/register"),
                    "response_types_supported": ["code"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                    "code_challenge_methods_supported": ["S256"],
                    "token_endpoint_auth_methods_supported": ["none"],
                }))
            }
        }
    };

    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(resource_metadata.clone()),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(server_metadata),
        )
        .route("/register", post(register))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/_stats", get(stats))
        .route("/_issue", post(issue))
        .merge(protected)
        .with_state(state)
}

#[derive(Deserialize)]
struct RegisterRequest {
    #[serde(default)]
    redirect_uris: Vec<String>,
}

async fn register(
    State(state): State<Shared>,
    Json(request): Json<RegisterRequest>,
) -> impl IntoResponse {
    state.registrations.fetch_add(1, Ordering::SeqCst);
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": "studio-test-client",
            "redirect_uris": request.redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })),
    )
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    redirect_uri: String,
    state: Option<String>,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    scope: String,
}

/// Behaves like a user who immediately approves: redirect back with a code.
async fn authorize(State(state): State<Shared>, Query(query): Query<AuthorizeQuery>) -> Redirect {
    let n = state.codes_issued.fetch_add(1, Ordering::SeqCst) + 1;
    *state.last_authorization.lock().unwrap() = (query.client_id.clone(), query.scope.clone());
    let mut target = format!("{}?code=code-{n}", query.redirect_uri);
    if let Some(value) = query.state {
        target.push_str(&format!("&state={value}"));
    }
    Redirect::to(&target)
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

async fn token(State(state): State<Shared>, Form(request): Form<TokenRequest>) -> Response {
    let (access, refresh) = match request.grant_type.as_str() {
        "authorization_code" => {
            let n = state.codes_issued.load(Ordering::SeqCst);
            (format!("access-{n}-0"), "refresh-1".to_owned())
        }
        "refresh_token" if request.refresh_token.as_deref() == Some("refresh-1") => {
            let n = state.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
            (format!("access-refreshed-{n}"), "refresh-1".to_owned())
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_grant"})),
            )
                .into_response();
        }
    };
    state.valid_tokens.lock().unwrap().push(access.clone());
    Json(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": 2,
        "refresh_token": refresh,
    }))
    .into_response()
}

/// Hands out an access token without the OAuth flow, for tests of other ways to obtain one (the
/// Azure login). Like every token, it replaces the previous one.
async fn issue(State(state): State<Shared>) -> Json<serde_json::Value> {
    let mut tokens = state.valid_tokens.lock().unwrap();
    let token = format!("issued-{}", tokens.len() + 1);
    tokens.push(token.clone());
    Json(json!({ "token": token }))
}

async fn stats(State(state): State<Shared>) -> Json<serde_json::Value> {
    Json(json!({
        "codesIssued": state.codes_issued.load(Ordering::SeqCst),
        "refreshes": state.refreshes.load(Ordering::SeqCst),
        "unauthorized": state.unauthorized.load(Ordering::SeqCst),
        "registrations": state.registrations.load(Ordering::SeqCst),
        "lastClientId": state.last_authorization.lock().unwrap().0,
        "lastScope": state.last_authorization.lock().unwrap().1,
    }))
}

async fn require_token(
    State((state, base)): State<(Shared, String)>,
    headers: HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Response {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    let valid = presented
        .as_ref()
        .is_some_and(|token| state.valid_tokens.lock().unwrap().contains(token));
    // Tokens expire after two seconds: the test server treats a token as valid only until refreshed
    // tokens replace it, so an expired one is rejected as soon as a newer token exists.
    let current = state.valid_tokens.lock().unwrap().last().cloned();
    if valid && presented == current {
        return next.run(request).await;
    }
    state.unauthorized.fetch_add(1, Ordering::SeqCst);
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            header::WWW_AUTHENTICATE,
            format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\""),
        )
        .body(Body::from("unauthorized"))
        .expect("static response")
}
