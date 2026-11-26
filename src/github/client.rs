use anyhow::Context;
use async_trait::async_trait;
use futures::{FutureExt, future::BoxFuture};
use http_body_util::BodyExt;
use http_body_util::Limited;
use reqwest::Body;
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::{Client, Request, RequestBuilder, Response, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use std::fmt::Debug;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing as log;
use url::Url;

use crate::jobs::Job;
use crate::utils::UrlExt;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use http::HeaderValue;
use octocrab::Octocrab;
use octocrab::models::InstallationId;

// TODO: Update to "2026-03-10" and see what breaks
// current version 2022-11-28 (supported until March 2028)
// Note: If you specify an API version that is no longer supported, you will receive a 410 Gone response.
// see: https://docs.github.com/rest/about-the-rest-api/api-versions?apiVersion=2026-03-10
const GITHUB_API_VERSION: &str = "2022-11-28";

const TRIAGEBOT_USER_AGENT: &str = "rust-lang-triagebot";

/// Finds the token in the user's environment, panicking if no suitable token
/// can be found.
fn default_app_auth_from_env() -> Option<GithubAuth> {
    match (
        std::env::var("GITHUB_APP_ID"),
        std::env::var("GITHUB_APP_PRIVATE_KEY"),
        std::env::var("GITHUB_APP_INSTALLATION_ID"),
    ) {
        (Ok(app_id), Ok(private_key), Ok(installation_id)) => {
            let app_id: u64 = app_id.parse().expect("GITHUB_APP_ID must be a number");
            let installation_id: u64 = installation_id
                .parse()
                .expect("GITHUB_APP_INSTALLATION_ID must be a number");
            let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key.as_bytes())
                .expect("GITHUB_APP_PRIVATE_KEY must be a valid RSA PEM key");
            tracing::info!(
                "Using GitHub App (app={app_id}, installation={installation_id}) for GitHub authentication"
            );

            Some(GithubAuth::App {
                app_auth: octocrab::auth::AppAuth {
                    app_id: octocrab::models::AppId(app_id),
                    key,
                },
                installation_id: InstallationId::from(installation_id),
                token_cache: Arc::new(std::sync::RwLock::new(None)),
            })
        }
        _ => None,
    }
}

fn default_token_auth_from_env() -> GithubAuth {
    let pat_token = std::env::var("GITHUB_TOKEN")
        // kept for retrocompatibility but usage is discouraged and will be deprecated
        .or_else(|_| std::env::var("GITHUB_API_TOKEN"))
        .or_else(|_| get_token_from_git_config())
        .expect("could not find token in GITHUB_TOKEN, GITHUB_API_TOKEN or .gitconfig/github.oath-token")
        .into();
    tracing::info!("Using PAT token for GitHub authentication");
    GithubAuth::Pat { token: pat_token }
}

fn get_token_from_git_config() -> anyhow::Result<String> {
    let output = std::process::Command::new("git")
        .arg("config")
        .arg("--get")
        .arg("github.oauth-token")
        .output()?;
    if !output.status.success() {
        anyhow::bail!("error received executing `git`: {:?}", output.status);
    }
    let git_token = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(git_token)
}

/// Cached GitHub App installation access token with its expiry time.
pub struct CachedInstallationToken {
    token: SecretString,
    expires_at: DateTime<Utc>,
}

/// Authentication mechanism for the GitHub API.
#[derive(Clone)]
pub enum GithubAuth {
    /// Classic Personal Access Token (PAT) authentication.
    /// Useful for local testing.
    Pat { token: SecretString },
    /// GitHub App authentication using app ID and private key.
    /// Generates JWTs to obtain short-lived installation access tokens.
    App {
        app_auth: octocrab::auth::AppAuth,
        installation_id: InstallationId,
        token_cache: Arc<std::sync::RwLock<Option<CachedInstallationToken>>>,
    },
}

pub(crate) struct RequestToken(SecretString);

impl RequestToken {
    fn auth_header(&self) -> reqwest::header::HeaderValue {
        let mut header =
            reqwest::header::HeaderValue::from_str(&format!("token {}", self.0.expose_secret()))
                .unwrap();
        header.set_sensitive(true);
        header
    }
}

/// GitHub repositories that opt into using triagebot with GitHub App authentication.
const REPOS_WITH_GITHUB_AUTH: &[&str] = &["rust-lang/triagebot", "rust-lang/bors-kindergarten"];

#[derive(Clone)]
pub struct GithubClient {
    auth: GithubAuth,
    /// Optional Github App auth used for selected repositories (if present).
    app_auth: Option<GithubAuth>,
    client: Client,
    api_url: Url,
    graphql_url: Url,
    raw_url: Url,
    /// If `true`, requests will sleep if it hits GitHub's rate limit.
    retry_rate_limit: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct RateLimitResources {
    pub core: RateLimit,
    pub search: RateLimit,
    pub graphql: RateLimit,
}

#[derive(Debug, serde::Deserialize)]
pub struct RateLimit {
    pub limit: u64,
    pub remaining: u64,
    pub reset: u64,
}

impl GithubClient {
    pub fn new(auth: GithubAuth, api_url: Url, graphql_url: Url, raw_url: Url) -> Self {
        GithubClient {
            client: Client::new(),
            auth,
            app_auth: default_app_auth_from_env(),
            api_url,
            graphql_url,
            raw_url,
            retry_rate_limit: false,
        }
    }

    pub fn new_from_env() -> Self {
        Self::new(
            default_token_auth_from_env(),
            Url::parse(
                &std::env::var("GITHUB_API_URL")
                    .unwrap_or_else(|_| "https://api.github.com".to_string()),
            )
            .expect("couldn't parse GITHUB_API_URL"),
            Url::parse(
                &std::env::var("GITHUB_GRAPHQL_API_URL")
                    .unwrap_or_else(|_| "https://api.github.com/graphql".to_string()),
            )
            .expect("couldn't parse GITHUB_GRAPHQL_API_URL"),
            Url::parse(
                &std::env::var("GITHUB_RAW_URL")
                    .unwrap_or_else(|_| "https://raw.githubusercontent.com".to_string()),
            )
            .expect("couldn't parse GITHUB_RAW_URL"),
        )
    }

    /// Ensures that a valid installation token is present for App auth.
    /// No-op for PAT auth.
    ///
    /// Fetches a new installation token if the cache is empty or the token
    /// is about to expire (within 5 minutes).
    async fn ensure_app_token(&self, request: &RequestBuilder) -> anyhow::Result<RequestToken> {
        // Temporary combination of two auth mechanisms:
        // If we figure out that we are working with a repo that opted into using Github App auth,
        // and that auth is present, use Github App auth.
        // Otherwise, use token auth.

        // Because we don't know what repository we are dealing with here, we have to resort to a
        // hack, and try to parse it from the URL.
        // But getting the URL from a `RequestBuilder` also isn't trivial, we have to first
        // build a cloned request out of it, and then get the URL.
        let url = request
            .try_clone()
            .and_then(|r| r.build().ok())
            .map(|r| r.url().clone());
        let auth = match (url, &self.app_auth) {
            (Some(url), Some(auth)) => {
                let repo = REPOS_WITH_GITHUB_AUTH
                    .iter()
                    .find(|r| url.path().contains(*r));
                if let Some(_repo) = repo {
                    // We found an opt-in repo, use Github App auth for it
                    auth
                } else {
                    &self.auth
                }
            }
            _ => &self.auth,
        };

        match &auth {
            GithubAuth::Pat { token } => Ok(RequestToken(token.clone())),
            GithubAuth::App {
                app_auth,
                installation_id,
                token_cache,
            } => {
                // Cached token is still valid at least for 5+ minutes
                if let Some(existing_token) = token_cache
                    .read()
                    .expect("token cache lock poisoned")
                    .as_ref()
                    .filter(|cached| cached.expires_at - Utc::now() >= chrono::Duration::minutes(5))
                {
                    return Ok(RequestToken(existing_token.token.clone()));
                }

                // Cached token is not valid, we need to get a new one
                let url = self.api_url_with_segments(&[
                    "app",
                    "installations",
                    &installation_id.to_string(),
                    "access_tokens",
                ]);
                let jwt = app_auth
                    .generate_bearer_token()
                    .context("failed to generate GitHub App JWT")?;
                let mut auth_header = HeaderValue::from_str(&format!("Bearer {jwt}"))?;
                auth_header.set_sensitive(true);
                let resp = self
                    .client
                    .post(url)
                    .header(USER_AGENT, TRIAGEBOT_USER_AGENT)
                    .header(AUTHORIZATION, auth_header)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .context("failed to request installation token")?
                    .error_for_status()
                    .context("GitHub rejected installation token request")?;

                #[derive(serde::Deserialize)]
                struct InstallationTokenResponse {
                    token: String,
                    expires_at: DateTime<Utc>,
                }
                let body: InstallationTokenResponse = resp
                    .json()
                    .await
                    .context("failed to parse installation token response")?;

                let token = SecretString::from(body.token.clone());
                {
                    let mut cache = token_cache.write().expect("token cache lock poisoned");
                    *cache = Some(CachedInstallationToken {
                        token,
                        expires_at: body.expires_at,
                    });
                }
                log::info!(
                    "Refreshed GitHub App installation token, expires at {}",
                    body.expires_at
                );
                Ok(RequestToken(SecretString::from(body.token)))
            }
        }
    }

    /// Create an Octocrab client with the current GitHub authentication.
    pub fn create_octocrab_client(&self) -> anyhow::Result<Octocrab> {
        let client = match &self.auth {
            GithubAuth::Pat { token } => octocrab::OctocrabBuilder::new()
                .personal_token(token.clone())
                .build()?,
            GithubAuth::App {
                app_auth,
                installation_id,
                ..
            } => {
                // We want a client scoped to a specific app installation
                octocrab::OctocrabBuilder::new()
                    .app(app_auth.app_id, app_auth.key.clone())
                    .build()?
                    .installation(*installation_id)?
            }
        };
        Ok(client)
    }

    pub(crate) fn api_url_with_segments<I>(&self, segments: I) -> Url
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        self.api_url.with_segments(segments)
    }

    pub(crate) fn raw_url_with_segments<I>(&self, segments: I) -> Url
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        self.raw_url.with_segments(segments)
    }

    /// Sets whether or not this client will retry when it hits GitHub's rate limit.
    ///
    /// Just beware that the retry may take a long time (like 30 minutes,
    /// depending on various factors).
    pub fn set_retry_rate_limit(&mut self, retry: bool) {
        self.retry_rate_limit = retry;
    }

    /// Send a request with the default response size limit.
    pub async fn send_req(&self, req: RequestBuilder) -> anyhow::Result<Bytes> {
        const MAX_DEFAULT_RESPONSE_SIZE: usize = 8 * 1024 * 1024; // 8 Mib

        self.send_req_with_limit(req, MAX_DEFAULT_RESPONSE_SIZE)
            .await
    }

    /// Send a request without checking response size, and return the raw response.
    /// The caller is responsible for checking whether the response was successful or not.
    pub async fn send_req_raw(&self, req: RequestBuilder) -> anyhow::Result<Response> {
        let token = self.ensure_app_token(&req).await?;
        let req = req.configure(token).build()?;
        Ok(self.client.execute(req).await?)
    }

    /// Send a request with a custom response size limit.
    pub async fn send_req_with_limit(
        &self,
        req: RequestBuilder,
        max_response_size: usize,
    ) -> anyhow::Result<Bytes> {
        const MAX_ATTEMPTS: u32 = 2;

        log::debug!("send_req with {:?}", req);

        let req_dbg = format!("{req:?}");

        let token = self.ensure_app_token(&req).await?;

        let req = req
            .configure(token)
            .build()
            .with_context(|| format!("building reqwest {req_dbg}"))?;

        let req_url = req.url().to_string();

        let mut resp = self.client.execute(req.try_clone().unwrap()).await?;
        if self.retry_rate_limit
            && let Some(sleep) = Self::needs_retry(&resp).await
        {
            resp = self.retry(req, sleep, MAX_ATTEMPTS).await?;
        }

        let maybe_err = resp.error_for_status_ref().err();
        let github_request_id = resp.headers().get("x-github-request-id").cloned();

        let resp: http::Response<Body> = resp.into();
        let limited = Limited::new(resp, max_response_size);

        let body = match limited.collect().await {
            Ok(body) => body.to_bytes(),
            Err(e) => match e.downcast::<http_body_util::LengthLimitError>() {
                Ok(e) => {
                    return Err(anyhow::Error::new(*e)).with_context(|| {
                            format!(
                                "req={req_url} (x-github-request-id: {}): max response size exceeded (over {max_response_size} bytes)",
                                github_request_id
                                    .as_ref()
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("unknown")
                            )
                        });
                }
                Err(e) => {
                    return Err(anyhow::Error::from_boxed(e)).with_context(|| {
                            format!(
                                "req={req_url} (x-github-request-id: {}): unable to complete the request",
                                github_request_id
                                    .as_ref()
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("unknown")
                            )
                        });
                }
            },
        };

        if let Some(e) = maybe_err {
            return Err(anyhow::Error::new(e)).with_context(|| {
                format!(
                    "req={req_url} (x-github-request-id: {}): {:.500}",
                    github_request_id
                        .as_ref()
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown"),
                    String::from_utf8_lossy(&body),
                )
            });
        }

        Ok(body)
    }

    async fn needs_retry(resp: &Response) -> Option<Duration> {
        const REMAINING: &str = "X-RateLimit-Remaining";
        const RESET: &str = "X-RateLimit-Reset";

        if !matches!(
            resp.status(),
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
        ) {
            return None;
        }

        let headers = resp.headers();
        if !(headers.contains_key(REMAINING) && headers.contains_key(RESET)) {
            return None;
        }

        let reset_time = headers[RESET].to_str().unwrap().parse::<u64>().unwrap();
        Some(Duration::from_secs(Self::calc_sleep(reset_time) + 10))
    }

    fn calc_sleep(reset_time: u64) -> u64 {
        let epoch_time = SystemTime::UNIX_EPOCH.elapsed().unwrap().as_secs();
        reset_time.saturating_sub(epoch_time)
    }

    fn retry(
        &self,
        req: Request,
        sleep: Duration,
        remaining_attempts: u32,
    ) -> BoxFuture<'_, Result<Response, anyhow::Error>> {
        #[derive(Debug, serde::Deserialize)]
        struct RateLimitResponse {
            resources: RateLimitResources,
        }

        log::warn!(
            "Retrying after {} seconds, remaining attepts {}",
            sleep.as_secs(),
            remaining_attempts,
        );

        async move {
            tokio::time::sleep(sleep).await;

            let rate_limit_req = self.client.get(format!("{}/rate_limit", self.api_url));
            let token = self.ensure_app_token(&rate_limit_req).await?;

            // check rate limit
            let rate_resp = self
                .client
                .execute(rate_limit_req.configure(token).build().unwrap())
                .await?;
            rate_resp.error_for_status_ref()?;
            let rate_limit_response = rate_resp.json::<RateLimitResponse>().await?;

            // Check url for search path because github has different rate limits for the search api
            let rate_limit = if req
                .url()
                .path_segments()
                .is_some_and(|mut segments| segments.next() == Some("search"))
            {
                rate_limit_response.resources.search
            } else {
                rate_limit_response.resources.core
            };

            // If we still don't have any more remaining attempts, try sleeping for the remaining
            // period of time
            if rate_limit.remaining == 0 {
                let sleep = Self::calc_sleep(rate_limit.reset);
                if sleep > 0 {
                    tokio::time::sleep(Duration::from_secs(sleep)).await;
                }
            }

            let resp = self.client.execute(req.try_clone().unwrap()).await?;
            if let Some(sleep) = Self::needs_retry(&resp).await
                && remaining_attempts > 0
            {
                return self.retry(req, sleep, remaining_attempts - 1).await;
            }

            Ok(resp)
        }
        .boxed()
    }

    pub async fn json<T>(&self, req: RequestBuilder) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let body = self.send_req(req).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    pub fn get(&self, url: Url) -> RequestBuilder {
        log::trace!("get {:?}", url);
        self.client.get(url)
    }

    pub fn patch(&self, url: Url) -> RequestBuilder {
        log::trace!("patch {:?}", url);
        self.client.patch(url)
    }

    pub fn delete(&self, url: Url) -> RequestBuilder {
        log::trace!("delete {:?}", url);
        self.client.delete(url)
    }

    pub fn post(&self, url: Url) -> RequestBuilder {
        log::trace!("post {:?}", url);
        self.client.post(url)
    }

    pub fn put(&self, url: Url) -> RequestBuilder {
        log::trace!("put {:?}", url);
        self.client.put(url)
    }

    /// Fetch current rate limit, remaining and used
    pub async fn rate_limit(&self) -> anyhow::Result<RateLimitResources> {
        #[derive(Debug, serde::Deserialize)]
        struct RateLimitResponse {
            resources: RateLimitResources,
        }

        let url = self.api_url_with_segments(["rate_limit"]);
        let response = self
            .json::<RateLimitResponse>(self.get(url))
            .await
            .context("failed to fetch GitHub /rate_limit endpoint")?;

        Ok(response.resources)
    }

    /// Issues an ad-hoc GraphQL query.
    ///
    /// You are responsible for checking the `errors` array when calling this
    /// function to determine if there is an error. Only use this if you are
    /// looking for specific error codes, or don't care about errors. Use
    /// [`GithubClient::graphql_query`] if you would prefer to have a generic
    /// error message.
    pub async fn graphql_query_with_errors(
        &self,
        query: &str,
        vars: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        // Our GraphQl query can end-up being quite big, let's set a higher default
        // response size than for normal REST Api response.
        const MAX_DEFAULT_GRAPH_QL_RESPONSE_SIZE: usize = 10 * 1024 * 1024;

        let body = self
            .send_req_with_limit(
                self.post(self.graphql_url.clone())
                    .json(&serde_json::json!({
                        "query": query,
                        "variables": vars,
                    })),
                MAX_DEFAULT_GRAPH_QL_RESPONSE_SIZE,
            )
            .await?;

        Ok(serde_json::from_slice(&body)?)
    }

    /// Issues an ad-hoc GraphQL query.
    ///
    /// See [`GithubClient::graphql_query_with_errors`] if you need to check
    /// for specific errors.
    pub async fn graphql_query(
        &self,
        query: &str,
        vars: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let mut result: serde_json::Value = self.graphql_query_with_errors(query, vars).await?;
        if let Some(errors) = result["errors"].take().as_array_mut() {
            anyhow::bail!(GraphQlErrors {
                errors: std::mem::take(errors)
                    .into_iter()
                    .map(|err| serde_json::from_value(err).unwrap_or_default())
                    .collect(),
            })
        }
        Ok(result)
    }
}

#[derive(Debug)]
pub struct GraphQlErrors {
    pub errors: Vec<GraphQlError>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct GraphQlError {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub path: Vec<String>,
    #[serde(default, rename = "type")]
    pub type_: String,
}

impl std::fmt::Display for GraphQlErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, err) in self.errors.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            f.write_str(&err.message)?;
        }
        Ok(())
    }
}

trait RequestSend: Sized {
    fn configure(self, token: RequestToken) -> Self;
}

impl RequestSend for RequestBuilder {
    fn configure(self, token: RequestToken) -> RequestBuilder {
        self.header(USER_AGENT, TRIAGEBOT_USER_AGENT)
            .header(AUTHORIZATION, token.auth_header())
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
    }
}

// Rate limit logging job

pub struct GithubRateLimitLoggingJob;

#[async_trait]
impl Job for GithubRateLimitLoggingJob {
    fn name(&self) -> &'static str {
        "rate_limit_logging_job"
    }

    async fn run(
        &self,
        ctx: &crate::handlers::Context,
        _metadata: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let rates = match ctx.github.rate_limit().await {
            Ok(rates) => rates,
            Err(err) => {
                tracing::error!("failed to fetch the current rate limits for Github: {err:?}");
                return Ok(());
            }
        };

        tracing::info!(
            "Github rate_limit: core={}/{} search={}/{} graphql={}/{}",
            rates.core.remaining,
            rates.core.limit,
            rates.search.remaining,
            rates.search.limit,
            rates.graphql.remaining,
            rates.graphql.limit
        );

        Ok(())
    }
}

#[test]
fn gh_api_version() {
    let c = GithubClient {
        auth: GithubAuth::Pat {
            token: Default::default(),
        },
        client: Client::new(),
        api_url: Url::parse("https://api.github.com").unwrap(),
        graphql_url: Url::parse("https://api.github.com/graphql").unwrap(),
        raw_url: Url::parse("https://raw.githubusercontent.com").unwrap(),
        retry_rate_limit: false,
    };

    let headers = c
        .get(Url::parse("http://www.example.com").unwrap())
        .configure(RequestToken(SecretString::default()))
        .build()
        .unwrap();
    let headers = headers
        .headers()
        .iter()
        .filter(|h| h.0 == "x-github-api-version")
        .map(|l| l.1)
        .collect::<Vec<_>>();
    assert_eq!(headers[0].to_str().unwrap(), "2022-11-28");
}
