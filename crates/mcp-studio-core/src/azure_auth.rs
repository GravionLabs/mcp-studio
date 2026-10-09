//! Silent sign-in for servers behind Microsoft Entra ID: the token comes from the user's Azure login
//! (Azure CLI, then Azure Developer CLI) instead of an OAuth flow with a client registered by us.
//!
//! The scope is read from the server's protected resource metadata. The token is kept in memory
//! only and fetched again shortly before it expires or when the server rejects it.

use std::{
    collections::HashMap,
    ffi::OsStr,
    process::{Output, Stdio},
    sync::Arc,
    time::Duration as StdDuration,
};

use azure_core::{
    credentials::TokenCredential,
    time::{Duration, OffsetDateTime},
};
use azure_identity::{DeveloperToolsCredential, DeveloperToolsCredentialOptions, Executor};
use futures_util::stream::BoxStream;
use http::{HeaderName, HeaderValue};
use rmcp::{
    model::ClientJsonRpcMessage,
    transport::streamable_http_client::{
        StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use serde::Deserialize;
use sse_stream::{Error as SseError, Sse};

use crate::{
    db::{DbError, DbResult},
    path_env,
};

/// A cached token is replaced once it has less than this left.
const REFRESH_MARGIN: Duration = Duration::minutes(5);

/// The credential chain of the Azure developer tools: `az`, then `azd`. They run with the `PATH` of
/// the user's login shell, because a desktop app starts with a minimal one and would not find them.
pub fn developer_tools() -> DbResult<Arc<dyn TokenCredential>> {
    let options = DeveloperToolsCredentialOptions {
        executor: Some(Arc::new(LoginPathExecutor)),
    };
    let credential = DeveloperToolsCredential::new(Some(options))
        .map_err(|e| DbError::Connection(format!("could not set up Azure credentials: {e}")))?;
    Ok(credential)
}

/// Runs the commands of the credential chain with the login shell's `PATH` added.
#[derive(Debug)]
struct LoginPathExecutor;

#[async_trait::async_trait]
impl Executor for LoginPathExecutor {
    async fn run(&self, program: &OsStr, args: &[&OsStr]) -> std::io::Result<Output> {
        let mut command = tokio::process::Command::new(program);
        command.args(args).stdin(Stdio::null()).kill_on_drop(true);
        if let Some(path) = search_path() {
            command.env("PATH", path);
        }
        command.output().await
    }
}

/// The login shell's `PATH` merged with ours; `None` where there is no login shell (Windows).
fn search_path() -> Option<String> {
    path_env::login_shell_path()
        .map(|login| path_env::merge_paths(login, &std::env::var("PATH").unwrap_or_default()))
}

/// How long to wait for the user to finish the sign-in in the browser.
const LOGIN_TIMEOUT: StdDuration = StdDuration::from_secs(5 * 60);

/// Signs in with the Azure CLI (`az login --allow-no-subscriptions`, in `tenant` if given). The CLI
/// opens the browser and returns when the user is done. The sign-in is the CLI's: later token
/// requests (and the user's other tools) use it.
pub async fn login(tenant: Option<&str>) -> DbResult<()> {
    login_with(az_program(), search_path(), tenant, LOGIN_TIMEOUT).await
}

fn az_program() -> &'static str {
    if cfg!(windows) {
        "az.cmd"
    } else {
        "az"
    }
}

/// Tenant IDs are GUIDs and tenant domains are DNS names; nothing else is passed to the CLI.
fn valid_tenant(tenant: &str) -> bool {
    !tenant.is_empty()
        && tenant.len() <= 253
        && tenant
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !tenant.starts_with('-')
}

async fn login_with(
    program: &str,
    search_path: Option<String>,
    tenant: Option<&str>,
    timeout: StdDuration,
) -> DbResult<()> {
    let tenant = tenant.map(str::trim).filter(|t| !t.is_empty());
    let mut command = tokio::process::Command::new(program);
    command.args(["login", "--allow-no-subscriptions"]);
    if let Some(tenant) = tenant {
        if !valid_tenant(tenant) {
            return Err(DbError::Invalid(
                "the tenant must be a tenant ID or a domain name".into(),
            ));
        }
        command.args(["--tenant", tenant]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(path) = search_path {
        command.env("PATH", path);
    }
    let child = command.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DbError::Connection(
                "the Azure CLI (`az`) was not found. Install it from https://aka.ms/installazurecli and try again"
                    .into(),
            )
        } else {
            DbError::Connection(format!("could not start the Azure CLI: {e}"))
        }
    })?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| {
            DbError::Connection(format!(
                "the Azure sign-in was not completed within {} minutes",
                timeout.as_secs().div_ceil(60)
            ))
        })?
        .map_err(|e| DbError::Connection(format!("the Azure CLI stopped unexpectedly: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(DbError::Connection(format!(
        "the Azure sign-in failed: {}",
        login_failure(&String::from_utf8_lossy(&output.stderr))
    )))
}

/// The part of the CLI's error output worth showing: its `ERROR:` lines, else the last lines.
fn login_failure(stderr: &str) -> String {
    let errors: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("ERROR:") || l.contains("AADSTS"))
        .collect();
    let lines = if errors.is_empty() {
        let all: Vec<&str> = stderr
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        all[all.len().saturating_sub(3)..].to_vec()
    } else {
        errors
    };
    let text = lines.join(" ");
    if text.is_empty() {
        "the Azure CLI gave no reason".into()
    } else {
        text
    }
}

/// Turns an error of the credential chain into something the user can act on.
pub fn explain(error: &azure_core::Error) -> String {
    let text = error.to_string();
    let lower = text.to_lowercase();
    if lower.contains("aadsts50076") || lower.contains("multi-factor") {
        return "your tenant requires multi-factor authentication. Use \"Sign in with Azure\" on the \
                server page and enter your tenant ID, or run `az login --tenant <tenant id> \
                --allow-no-subscriptions` and complete the prompt in the browser"
            .into();
    }
    if lower.contains("az login")
        || lower.contains("azd auth login")
        || lower.contains("not logged in")
    {
        return "you are not signed in to Azure. Use \"Sign in with Azure\" on the server page, or run \
                `az login --allow-no-subscriptions`"
            .into();
    }
    if lower.contains("not found")
        || lower.contains("no such file")
        || lower.contains("is not recognized")
    {
        return "neither the Azure CLI (`az`) nor the Azure Developer CLI (`azd`) was found. \
                Install one of them and sign in"
            .into();
    }
    text
}

/// Hands out tokens for one scope and keeps the current one until shortly before it expires.
pub struct TokenCache {
    credential: Arc<dyn TokenCredential>,
    scopes: Vec<String>,
    current: tokio::sync::Mutex<Option<(String, OffsetDateTime)>>,
}

impl TokenCache {
    pub fn new(credential: Arc<dyn TokenCredential>, scopes: Vec<String>) -> Self {
        Self {
            credential,
            scopes,
            current: tokio::sync::Mutex::new(None),
        }
    }

    /// A token that is valid for at least a few more minutes.
    pub async fn token(&self) -> DbResult<String> {
        let mut current = self.current.lock().await;
        if let Some((token, expires_on)) = current.as_ref() {
            if *expires_on - OffsetDateTime::now_utc() > REFRESH_MARGIN {
                return Ok(token.clone());
            }
        }
        let scopes: Vec<&str> = self.scopes.iter().map(String::as_str).collect();
        let fresh = self
            .credential
            .get_token(&scopes, None)
            .await
            .map_err(|e| DbError::Connection(format!("Azure sign-in failed: {}", explain(&e))))?;
        let token = fresh.token.secret().to_owned();
        *current = Some((token.clone(), fresh.expires_on));
        Ok(token)
    }

    /// Drops the cached token, e.g. after the server rejected it.
    pub async fn invalidate(&self) {
        *self.current.lock().await = None;
    }
}

#[derive(Deserialize)]
struct ResourceMetadata {
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    authorization_servers: Vec<String>,
}

/// Hosts of Microsoft's identity platform.
const ENTRA_HOSTS: [&str; 3] = [
    "login.microsoftonline.com",
    "login.windows.net",
    "sts.windows.net",
];

/// How long to wait for the metadata when only looking for a hint.
const DETECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn is_entra(authorization_server: &str) -> bool {
    url::Url::parse(authorization_server)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| ENTRA_HOSTS.contains(&host.as_str()))
}

/// Whether the server's protected resource metadata names Microsoft Entra ID as its authorization
/// server. Any failure to find out (network, no metadata) counts as "no".
pub async fn uses_entra(url: &str) -> bool {
    let Ok(metadata_url) = metadata_url(url) else {
        return false;
    };
    let Ok(http) = reqwest::Client::builder().timeout(DETECT_TIMEOUT).build() else {
        return false;
    };
    let Ok(response) = http.get(metadata_url).send().await else {
        return false;
    };
    match response.json::<ResourceMetadata>().await {
        Ok(metadata) => metadata.authorization_servers.iter().any(|s| is_entra(s)),
        Err(_) => false,
    }
}

/// Whether `url` is an Azure DevOps MCP server.
pub fn is_azure_devops(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "mcp.dev.azure.com")
}

/// The URL of the protected resource metadata for `url` (RFC 9728): the well-known path is
/// inserted between the host and the path of the resource.
fn metadata_url(url: &str) -> DbResult<String> {
    let parsed = url::Url::parse(url).map_err(|e| DbError::Invalid(format!("invalid URL: {e}")))?;
    let path = parsed.path().trim_end_matches('/');
    let mut metadata = parsed.clone();
    metadata.set_path(&format!("/.well-known/oauth-protected-resource{path}"));
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata.into())
}

/// The scopes to ask the credential for: the override if there is one, otherwise what the server
/// advertises in its protected resource metadata.
pub async fn resolve_scopes(
    http: &reqwest::Client,
    url: &str,
    configured: Option<&str>,
) -> DbResult<Vec<String>> {
    if let Some(configured) = configured {
        let scopes: Vec<String> = configured.split_whitespace().map(str::to_owned).collect();
        if !scopes.is_empty() {
            return Ok(scopes);
        }
    }
    let metadata_url = metadata_url(url)?;
    let missing = |why: &str| {
        DbError::Connection(format!(
            "could not find out which scope the server needs ({why}). Enter it in the server's scopes field"
        ))
    };
    let response = http
        .get(&metadata_url)
        .send()
        .await
        .map_err(|e| missing(&e.to_string()))?;
    if !response.status().is_success() {
        return Err(missing(&format!(
            "{metadata_url} answered {}",
            response.status()
        )));
    }
    let metadata: ResourceMetadata = response.json().await.map_err(|e| missing(&e.to_string()))?;
    match metadata.scopes_supported.first() {
        Some(scope) => Ok(vec![scope.clone()]),
        None => Err(missing("the server advertises no scopes")),
    }
}

/// An HTTP client for the Streamable HTTP transport that signs every request with a token from the
/// Azure login and, if the server answers 401, fetches a new token and tries once more.
#[derive(Clone)]
pub struct AzureAuthClient {
    http: reqwest::Client,
    tokens: Arc<TokenCache>,
}

impl AzureAuthClient {
    pub fn new(http: reqwest::Client, tokens: Arc<TokenCache>) -> Self {
        Self { http, tokens }
    }

    async fn call<T, F, Fut>(&self, call: F) -> Result<T, StreamableHttpError<reqwest::Error>>
    where
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, StreamableHttpError<reqwest::Error>>>,
    {
        let token = self.token().await?;
        match call(token).await {
            Err(StreamableHttpError::AuthRequired(_)) => {
                self.tokens.invalidate().await;
                call(self.token().await?).await
            }
            result => result,
        }
    }

    async fn token(&self) -> Result<String, StreamableHttpError<reqwest::Error>> {
        self.tokens
            .token()
            .await
            .map_err(|e| StreamableHttpError::Io(std::io::Error::other(e.to_string())))
    }
}

type Headers = HashMap<HeaderName, HeaderValue>;

impl StreamableHttpClient for AzureAuthClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: Headers,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.call(|token| {
            self.http.post_message(
                uri.clone(),
                message.clone(),
                session_id.clone(),
                Some(token),
                custom_headers.clone(),
            )
        })
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: Headers,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.call(|token| {
            self.http.post_message_with_max_sse_event_size(
                uri.clone(),
                message.clone(),
                session_id.clone(),
                Some(token),
                custom_headers.clone(),
                max_sse_event_size,
            )
        })
        .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        _auth_header: Option<String>,
        custom_headers: Headers,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.call(|token| {
            self.http.delete_session(
                uri.clone(),
                session_id.clone(),
                Some(token),
                custom_headers.clone(),
            )
        })
        .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: Headers,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.call(|token| {
            self.http.get_stream(
                uri.clone(),
                session_id.clone(),
                last_event_id.clone(),
                Some(token),
                custom_headers.clone(),
            )
        })
        .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: Headers,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.call(|token| {
            self.http.get_stream_with_max_sse_event_size(
                uri.clone(),
                session_id.clone(),
                last_event_id.clone(),
                Some(token),
                custom_headers.clone(),
                max_sse_event_size,
            )
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use azure_core::{credentials::AccessToken, error::ErrorKind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct FakeCredential {
        calls: AtomicUsize,
        lifetime: Duration,
        error: Option<&'static str>,
        scopes: std::sync::Mutex<Vec<String>>,
    }

    impl FakeCredential {
        fn new(lifetime: Duration) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                lifetime,
                error: None,
                scopes: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn failing(message: &'static str) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                lifetime: Duration::hours(1),
                error: Some(message),
                scopes: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl TokenCredential for FakeCredential {
        async fn get_token(
            &self,
            scopes: &[&str],
            _options: Option<azure_core::credentials::TokenRequestOptions<'_>>,
        ) -> azure_core::Result<AccessToken> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            *self.scopes.lock().unwrap() = scopes.iter().map(|s| (*s).to_owned()).collect();
            if let Some(message) = self.error {
                return Err(azure_core::Error::with_message(
                    ErrorKind::Credential,
                    message,
                ));
            }
            Ok(AccessToken::new(
                format!("token-{n}"),
                OffsetDateTime::now_utc() + self.lifetime,
            ))
        }
    }

    fn cache(credential: Arc<FakeCredential>) -> TokenCache {
        TokenCache::new(credential, vec!["https://example.test/.default".into()])
    }

    #[tokio::test]
    async fn a_token_is_reused_until_shortly_before_it_expires() {
        let credential = FakeCredential::new(Duration::hours(1));
        let tokens = cache(credential.clone());
        assert_eq!(tokens.token().await.unwrap(), "token-1");
        assert_eq!(tokens.token().await.unwrap(), "token-1");
        assert_eq!(credential.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *credential.scopes.lock().unwrap(),
            vec!["https://example.test/.default".to_owned()]
        );
    }

    #[tokio::test]
    async fn a_token_that_is_about_to_expire_is_replaced() {
        let credential = FakeCredential::new(Duration::minutes(2));
        let tokens = cache(credential.clone());
        assert_eq!(tokens.token().await.unwrap(), "token-1");
        assert_eq!(tokens.token().await.unwrap(), "token-2");
    }

    #[tokio::test]
    async fn an_invalidated_token_is_fetched_again() {
        let credential = FakeCredential::new(Duration::hours(1));
        let tokens = cache(credential.clone());
        assert_eq!(tokens.token().await.unwrap(), "token-1");
        tokens.invalidate().await;
        assert_eq!(tokens.token().await.unwrap(), "token-2");
    }

    #[tokio::test]
    async fn credential_errors_are_explained() {
        for (message, expected) in [
            (
                "AADSTS50076: you must use multi-factor authentication",
                "multi-factor",
            ),
            (
                "Please run 'az login' to set up an account",
                "not signed in",
            ),
            ("program not found", "Azure CLI"),
        ] {
            let tokens = cache(FakeCredential::failing(message));
            let error = tokens.token().await.unwrap_err().to_string();
            assert!(error.contains("Azure sign-in failed"), "{error}");
            assert!(error.contains(expected), "{message}: {error}");
        }
        let tokens = cache(FakeCredential::failing("something else"));
        assert!(tokens
            .token()
            .await
            .unwrap_err()
            .to_string()
            .contains("something else"));
    }

    #[test]
    fn the_metadata_url_follows_rfc_9728() {
        assert_eq!(
            metadata_url("https://mcp.dev.azure.com/gravionlabs").unwrap(),
            "https://mcp.dev.azure.com/.well-known/oauth-protected-resource/gravionlabs"
        );
        assert_eq!(
            metadata_url("https://example.test/").unwrap(),
            "https://example.test/.well-known/oauth-protected-resource"
        );
        assert_eq!(
            metadata_url("https://example.test/mcp/?x=1#f").unwrap(),
            "https://example.test/.well-known/oauth-protected-resource/mcp"
        );
    }

    #[test]
    fn microsoft_identity_hosts_are_recognized() {
        assert!(is_entra(
            "https://login.microsoftonline.com/organizations/v2.0"
        ));
        assert!(is_entra("https://LOGIN.windows.net/tenant"));
        assert!(!is_entra("https://login.microsoftonline.com.evil.test/x"));
        assert!(!is_entra("https://accounts.example.test"));
        assert!(!is_entra("not a url"));
        assert!(is_azure_devops("https://mcp.dev.azure.com/org"));
        assert!(!is_azure_devops("https://example.test/mcp.dev.azure.com"));
    }

    #[tokio::test]
    async fn configured_scopes_skip_the_discovery() {
        let scopes = resolve_scopes(
            &reqwest::Client::new(),
            "https://unreachable.invalid/mcp",
            Some(" a/.default  b "),
        )
        .await
        .unwrap();
        assert_eq!(scopes, vec!["a/.default", "b"]);
    }

    #[test]
    fn tenants_are_ids_or_domain_names_only() {
        assert!(valid_tenant("72f988bf-86f1-41af-91ab-2d7cd011db47"));
        assert!(valid_tenant("contoso.onmicrosoft.com"));
        for bad in ["", "-x", "a b", "a;rm -rf", "a&b", "$(x)", "a\"b"] {
            assert!(!valid_tenant(bad), "{bad}");
        }
    }

    #[test]
    fn a_failed_login_shows_the_error_lines_of_the_cli() {
        let stderr = "WARNING: something\nERROR: AADSTS50076: MFA needed\nTrace ID: x\n";
        assert_eq!(login_failure(stderr), "ERROR: AADSTS50076: MFA needed");
        assert_eq!(login_failure("a\nb\nc\nd\n"), "b c d");
        assert_eq!(login_failure(""), "the Azure CLI gave no reason");
    }

    #[test]
    fn explanations_point_to_the_sign_in_button() {
        let not_signed_in =
            azure_core::Error::with_message(ErrorKind::Credential, "Please run 'az login'");
        let message = explain(&not_signed_in);
        assert!(message.contains("Sign in with Azure"), "{message}");
        assert!(message.contains("az login"), "{message}");
        let mfa = azure_core::Error::with_message(ErrorKind::Credential, "AADSTS50076");
        assert!(explain(&mfa).contains("multi-factor"));
    }

    #[tokio::test]
    async fn login_reports_a_missing_azure_cli() {
        let error = login_with(
            "mcp-studio-no-such-program",
            Some(String::new()),
            None,
            StdDuration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Azure CLI"), "{error}");
        assert!(error.to_string().contains("not found"), "{error}");
    }

    #[tokio::test]
    async fn login_rejects_a_tenant_that_is_not_one() {
        let error = login_with("az", None, Some("a; b"), StdDuration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Invalid(_)), "{error}");
    }

    /// `login_with`, tried again while the kernel still considers the freshly written script open
    /// for writing (`ETXTBSY`): another test thread may have forked while it was being written.
    #[cfg(unix)]
    async fn login_retrying(
        path: &Option<String>,
        tenant: Option<&str>,
        timeout: StdDuration,
    ) -> DbResult<()> {
        let mut result = login_with("az", path.clone(), tenant, timeout).await;
        for _ in 0..20 {
            match &result {
                Err(e) if e.to_string().contains("Text file busy") => {
                    tokio::time::sleep(StdDuration::from_millis(50)).await;
                    result = login_with("az", path.clone(), tenant, timeout).await;
                }
                _ => break,
            }
        }
        result
    }

    /// A stand-in `az` that records its arguments and exits with the given code.
    #[cfg(unix)]
    fn fake_az(dir: &std::path::Path, exit: i32, stderr: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let record = dir.join("args.txt");
        let script = dir.join("az");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$@\" > '{}'\necho '{stderr}' >&2\nexit {exit}\n",
                record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        record
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn login_runs_az_login_found_through_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let record = fake_az(dir.path(), 0, "WARNING: noise");
        let path = Some(dir.path().to_string_lossy().into_owned());

        login_retrying(&path, None, StdDuration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&record).unwrap().trim(),
            "login --allow-no-subscriptions"
        );

        login_retrying(
            &path,
            Some(" contoso.onmicrosoft.com "),
            StdDuration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&record).unwrap().trim(),
            "login --allow-no-subscriptions --tenant contoso.onmicrosoft.com"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn login_reports_why_the_cli_failed() {
        let dir = tempfile::tempdir().unwrap();
        fake_az(dir.path(), 1, "ERROR: user cancelled");
        let path = Some(dir.path().to_string_lossy().into_owned());

        let error = login_retrying(&path, None, StdDuration::from_secs(5))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("sign-in failed"), "{error}");
        assert!(error.to_string().contains("user cancelled"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn login_gives_up_when_the_user_never_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("az");
        std::fs::write(&script, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = Some(dir.path().to_string_lossy().into_owned());

        let error = login_retrying(&path, None, StdDuration::from_millis(200))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("not completed"), "{error}");
    }
}
