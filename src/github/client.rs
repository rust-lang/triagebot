/// Finds the token in the user's environment, panicking if no suitable token
/// can be found.
pub fn default_token_from_env() -> SecretString {
    std::env::var("GITHUB_TOKEN")
        // kept for retrocompatibility but usage is discouraged and will be deprecated
        .or_else(|_| std::env::var("GITHUB_API_TOKEN"))
        .or_else(|_| get_token_from_git_config())
        .expect("could not find token in GITHUB_TOKEN, GITHUB_API_TOKEN or .gitconfig/github.oath-token")
        .into()
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

#[derive(Clone)]
pub struct GithubClient {
    token: SecretString,
    client: Client,
    api_url: String,
    graphql_url: String,
    raw_url: String,
    /// If `true`, requests will sleep if it hits GitHub's rate limit.
    retry_rate_limit: bool,
}

impl GithubClient {
    pub fn new(token: SecretString, api_url: String, graphql_url: String, raw_url: String) -> Self {
        GithubClient {
            client: Client::new(),
            token,
            api_url,
            graphql_url,
            raw_url,
            retry_rate_limit: false,
        }
    }

    pub fn new_from_env() -> Self {
        Self::new(
            default_token_from_env(),
            std::env::var("GITHUB_API_URL")
                .unwrap_or_else(|_| "https://api.github.com".to_string()),
            std::env::var("GITHUB_GRAPHQL_API_URL")
                .unwrap_or_else(|_| "https://api.github.com/graphql".to_string()),
            std::env::var("GITHUB_RAW_URL")
                .unwrap_or_else(|_| "https://raw.githubusercontent.com".to_string()),
        )
    }

    /// Sets whether or not this client will retry when it hits GitHub's rate limit.
    ///
    /// Just beware that the retry may take a long time (like 30 minutes,
    /// depending on various factors).
    pub fn set_retry_rate_limit(&mut self, retry: bool) {
        self.retry_rate_limit = retry;
    }

    pub fn raw(&self) -> &Client {
        &self.client
    }

    async fn send_req(&self, req: RequestBuilder) -> anyhow::Result<(Bytes, String)> {
        const MAX_ATTEMPTS: u32 = 2;
        log::debug!("send_req with {:?}", req);
        let req_dbg = format!("{req:?}");
        let req = req
            .build()
            .with_context(|| format!("building reqwest {req_dbg}"))?;

        let mut resp = self.client.execute(req.try_clone().unwrap()).await?;
        if self.retry_rate_limit
            && let Some(sleep) = Self::needs_retry(&resp).await
        {
            resp = self.retry(req, sleep, MAX_ATTEMPTS).await?;
        }
        let maybe_err = resp.error_for_status_ref().err();
        let body = resp
            .bytes()
            .await
            .with_context(|| format!("failed to read response body {req_dbg}"))?;
        if let Some(e) = maybe_err {
            return Err(anyhow::Error::new(e))
                .with_context(|| format!("response: {}", String::from_utf8_lossy(&body)));
        }

        Ok((body, req_dbg))
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
    ) -> BoxFuture<'_, Result<Response, reqwest::Error>> {
        #[derive(Debug, serde::Deserialize)]
        struct RateLimit {
            #[allow(unused)]
            pub limit: u64,
            pub remaining: u64,
            pub reset: u64,
        }

        #[derive(Debug, serde::Deserialize)]
        struct RateLimitResponse {
            pub resources: Resources,
        }

        #[derive(Debug, serde::Deserialize)]
        struct Resources {
            pub core: RateLimit,
            pub search: RateLimit,
            #[allow(unused)]
            pub graphql: RateLimit,
            #[allow(unused)]
            pub source_import: RateLimit,
        }

        log::warn!(
            "Retrying after {} seconds, remaining attepts {}",
            sleep.as_secs(),
            remaining_attempts,
        );

        async move {
            tokio::time::sleep(sleep).await;

            // check rate limit
            let rate_resp = self
                .client
                .execute(
                    self.client
                        .get(format!("{}/rate_limit", self.api_url))
                        .configure(self)
                        .build()
                        .unwrap(),
                )
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
        let (body, _req_dbg) = self.send_req(req).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    fn get(&self, url: &str) -> RequestBuilder {
        log::trace!("get {:?}", url);
        self.client.get(url).configure(self)
    }

    fn patch(&self, url: &str) -> RequestBuilder {
        log::trace!("patch {:?}", url);
        self.client.patch(url).configure(self)
    }

    fn delete(&self, url: &str) -> RequestBuilder {
        log::trace!("delete {:?}", url);
        self.client.delete(url).configure(self)
    }

    fn post(&self, url: &str) -> RequestBuilder {
        log::trace!("post {:?}", url);
        self.client.post(url).configure(self)
    }

    #[allow(unused)]
    fn put(&self, url: &str) -> RequestBuilder {
        log::trace!("put {:?}", url);
        self.client.put(url).configure(self)
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
        self.json(self.post(&self.graphql_url).json(&serde_json::json!({
            "query": query,
            "variables": vars,
        })))
        .await
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
        let result: serde_json::Value = self.graphql_query_with_errors(query, vars).await?;
        if let Some(errors) = result["errors"].as_array() {
            let messages = errors
                .iter()
                .map(|err| err["message"].as_str().unwrap_or_default())
                .format("\n");
            anyhow::bail!("error: {messages}");
        }
        Ok(result)
    }
}

trait RequestSend: Sized {
    fn configure(self, g: &GithubClient) -> Self;
}

impl RequestSend for RequestBuilder {
    fn configure(self, g: &GithubClient) -> RequestBuilder {
        let mut auth = reqwest::header::HeaderValue::from_maybe_shared(format!(
            "token {}",
            g.token.expose_secret()
        ))
        .unwrap();
        auth.set_sensitive(true);
        self.header(USER_AGENT, "rust-lang-triagebot")
            .header(AUTHORIZATION, &auth)
    }
}
