//! OAuth 2.1 for Streamable HTTP servers: browser sign-in with PKCE, tokens in the OS keyring,
//! automatic refresh.
//!
//! The protocol work (discovery, dynamic client registration, PKCE, token exchange and refresh) is
//! done by `rmcp`. This module adds what the app must provide: a loopback redirect listener, a
//! credential store backed by the keyring, and the glue that turns a signed-in server into a transport.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use rmcp::transport::{
    auth::{
        AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession,
    },
    CredentialStore, StoredCredentials,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

use crate::{
    db::{DbError, DbResult},
    secrets::SecretStore,
};

/// How long the user has to finish signing in.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// Name of the keyring entry that holds a server's OAuth credentials.
pub fn credential_key(server_id: &str) -> String {
    format!("oauth/{server_id}")
}

/// Stores `rmcp`'s credentials as one JSON document in the OS keyring.
#[derive(Clone)]
pub struct KeyringCredentialStore {
    secrets: Arc<dyn SecretStore>,
    key: String,
}

impl KeyringCredentialStore {
    pub fn new(secrets: Arc<dyn SecretStore>, server_id: &str) -> Self {
        Self {
            secrets,
            key: credential_key(server_id),
        }
    }

    fn store_error(error: impl std::fmt::Display) -> AuthError {
        AuthError::InternalError(format!("credential store: {error}"))
    }
}

#[async_trait]
impl CredentialStore for KeyringCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let (secrets, key) = (self.secrets.clone(), self.key.clone());
        let text = tokio::task::spawn_blocking(move || secrets.get(&key))
            .await
            .map_err(Self::store_error)?
            .map_err(Self::store_error)?;
        text.map(|t| serde_json::from_str(&t).map_err(Self::store_error))
            .transpose()
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let json = serde_json::to_string(&credentials).map_err(Self::store_error)?;
        let (secrets, key) = (self.secrets.clone(), self.key.clone());
        tokio::task::spawn_blocking(move || secrets.set(&key, &json))
            .await
            .map_err(Self::store_error)?
            .map_err(Self::store_error)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let (secrets, key) = (self.secrets.clone(), self.key.clone());
        tokio::task::spawn_blocking(move || secrets.delete(&key))
            .await
            .map_err(Self::store_error)?
            .map_err(Self::store_error)
    }
}

/// Opens the authorization page in the user's browser.
pub trait UrlOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), String>;
}

impl<F> UrlOpener for F
where
    F: Fn(&str) -> Result<(), String> + Send + Sync,
{
    fn open(&self, url: &str) -> Result<(), String> {
        self(url)
    }
}

fn oauth_error(context: &str, error: AuthError) -> DbError {
    DbError::Connection(format!("{context}: {error}"))
}

/// True if valid-looking credentials (with a token) are stored for the server.
pub async fn is_signed_in(store: &KeyringCredentialStore) -> bool {
    matches!(store.load().await, Ok(Some(c)) if c.token_response.is_some())
}

/// Removes the stored credentials.
pub async fn sign_out(store: &KeyringCredentialStore) -> DbResult<()> {
    store
        .clear()
        .await
        .map_err(|e| oauth_error("could not remove the credentials", e))
}

/// What the user configured for a server's sign-in beyond its URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignInSettings {
    /// A client ID registered by hand. Without one the app registers itself dynamically.
    pub client_id: Option<String>,
    /// Space-separated scopes to request. Without any, the server's advertised ones are used.
    pub scopes: Option<String>,
    /// Fixed port of the loopback redirect. Without one a free port is picked.
    pub callback_port: Option<u16>,
}

impl SignInSettings {
    /// The host of the redirect URI. Microsoft Entra ID only accepts `localhost` for the loopback
    /// redirect of a registered app, so a hand-registered client uses it; the listener is on 127.0.0.1.
    fn redirect_host(&self) -> &'static str {
        if self.client_id.is_some() {
            "localhost"
        } else {
            "127.0.0.1"
        }
    }

    fn scope_list(&self) -> Vec<String> {
        self.scopes
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
}

/// Runs the browser sign-in for the server at `url` and stores the tokens.
pub async fn sign_in(
    url: &str,
    settings: &SignInSettings,
    store: KeyringCredentialStore,
    opener: &dyn UrlOpener,
    timeout: Duration,
) -> DbResult<()> {
    let wanted_port = settings.callback_port.unwrap_or(0);
    let listener = TcpListener::bind(("127.0.0.1", wanted_port))
        .await
        .map_err(|e| {
            DbError::Connection(format!(
                "could not listen on port {wanted_port} for the sign-in redirect: {e}"
            ))
        })?;
    let port = listener.local_addr()?.port();
    let origin = format!("http://{}:{port}", settings.redirect_host());
    let redirect_uri = format!("{origin}/callback");

    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|e| oauth_error("could not start the sign-in", e))?;
    manager.set_credential_store(store);
    let resolution = manager
        .resolve_metadata()
        .await
        .map_err(|e| oauth_error("could not discover the authorization server", e))?;
    manager.set_metadata(resolution.metadata);

    let mut request = AuthorizationRequest::new(redirect_uri)
        .with_client_name("MCP Studio")
        .with_scopes(settings.scope_list());
    if let Some(client_id) = &settings.client_id {
        request = request.with_preregistered_client(client_id);
    }
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, e)| oauth_error("could not prepare the authorization", e))?;

    opener
        .open(session.get_authorization_url())
        .map_err(|e| DbError::Connection(format!("could not open the browser: {e}")))?;

    let target = tokio::time::timeout(timeout, wait_for_redirect(listener))
        .await
        .map_err(|_| DbError::Connection("sign-in timed out; try again".into()))??;
    session
        .handle_callback_url(&format!("{origin}{target}"))
        .await
        .map_err(|e| oauth_error("sign-in failed", e))?;
    Ok(())
}

/// Builds an authorized HTTP client for a signed-in server. Tokens are refreshed automatically and
/// written back to the store.
pub async fn auth_client(
    url: &str,
    store: KeyringCredentialStore,
) -> DbResult<AuthClient<reqwest::Client>> {
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|e| oauth_error("could not set up authorization", e))?;
    manager.set_credential_store(store);
    let has_credentials = manager
        .initialize_from_store()
        .await
        .map_err(|e| oauth_error("could not read the stored credentials", e))?;
    if !has_credentials {
        return Err(DbError::Connection(
            "sign-in required: this server needs you to sign in first".into(),
        ));
    }
    Ok(AuthClient::new(reqwest::Client::new(), manager))
}

/// Waits for the browser to come back to `/callback` and returns the request target
/// (`/callback?code=...&state=...`). Other requests (e.g. the favicon) are answered with 404.
async fn wait_for_redirect(listener: TcpListener) -> DbResult<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut reader = BufReader::new(&mut stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).await.is_err() {
            continue;
        }
        let target = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_owned();
        if !target.starts_with("/callback") {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let page = "<!doctype html><meta charset=utf-8><title>MCP Studio</title>\
            <body style=\"font-family:system-ui;padding:2rem\"><h2>Signed in</h2>\
            <p>You can close this tab and return to MCP Studio.</p></body>";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
        return Ok(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[test]
    fn credential_keys_are_namespaced_per_server() {
        assert_eq!(credential_key("abc"), "oauth/abc");
    }

    #[tokio::test]
    async fn the_store_roundtrips_through_the_secret_store() {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
        let store = KeyringCredentialStore::new(secrets.clone(), "srv");
        assert!(store.load().await.unwrap().is_none());
        assert!(!is_signed_in(&store).await);

        let credentials =
            StoredCredentials::new("client".into(), None, vec!["read".into()], Some(1));
        store.save(credentials).await.unwrap();
        let loaded = store.load().await.unwrap().unwrap();
        assert_eq!(loaded.client_id, "client");
        assert_eq!(loaded.granted_scopes, vec!["read"]);
        // Credentials without a token do not count as signed in.
        assert!(!is_signed_in(&store).await);
        assert!(secrets.get("oauth/srv").unwrap().is_some());

        sign_out(&store).await.unwrap();
        assert!(store.load().await.unwrap().is_none());
    }

    #[test]
    fn a_registered_client_redirects_to_localhost_and_scopes_are_split() {
        let dynamic = SignInSettings::default();
        assert_eq!(dynamic.redirect_host(), "127.0.0.1");
        assert!(dynamic.scope_list().is_empty());

        let entra = SignInSettings {
            client_id: Some("abc".into()),
            scopes: Some("  api://x/.default   offline_access ".into()),
            callback_port: Some(3118),
        };
        assert_eq!(entra.redirect_host(), "localhost");
        assert_eq!(entra.scope_list(), ["api://x/.default", "offline_access"]);
    }

    #[tokio::test]
    async fn the_redirect_listener_answers_and_ignores_other_paths() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(wait_for_redirect(listener));

        let favicon = reqwest::get(format!("http://127.0.0.1:{port}/favicon.ico"))
            .await
            .unwrap();
        assert_eq!(favicon.status(), 404);
        let response = reqwest::get(format!(
            "http://127.0.0.1:{port}/callback?code=abc&state=xyz"
        ))
        .await
        .unwrap();
        assert!(response.status().is_success());
        assert!(response.text().await.unwrap().contains("Signed in"));
        assert_eq!(
            waiter.await.unwrap().unwrap(),
            "/callback?code=abc&state=xyz"
        );
    }

    #[tokio::test]
    async fn an_unauthenticated_server_reports_that_sign_in_is_required() {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
        let store = KeyringCredentialStore::new(secrets, "srv");
        let error = auth_client("http://127.0.0.1:9/mcp", store)
            .await
            .map(|_| ())
            .expect_err("not signed in");
        assert!(error.to_string().contains("sign-in required"), "{error}");
    }
}
