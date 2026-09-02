use reqwest::Client;
use reqwest::header;
use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::env;
use std::sync::OnceLock;

// OpenAPI spec: https://crates.io/api/openapi.json
const CRATES_IO_BASE_URL: &str = "https://crates.io/api/v1";

/// Access to the Crates.io API
pub struct CratesIoApi {
    client: Client,
    // The token is loaded lazily, to avoid requiring the API token if crates.io APIs are not
    // actually accessed.
    token: OnceLock<SecretString>,
}

impl CratesIoApi {
    pub fn new() -> Self {
        let mut map = HeaderMap::default();
        map.insert(
            header::USER_AGENT,
            HeaderValue::from_static("triagebot@rust-lang.org"),
        );

        Self {
            client: reqwest::ClientBuilder::default()
                .default_headers(map)
                .build()
                .unwrap(),
            token: Default::default(),
        }
    }

    fn get_api_token(&self) -> &SecretString {
        self.token.get_or_init(|| {
            env::var("CRATES_IO_API_TOKEN")
                .expect("CRATES_IO_API_TOKEN is missing")
                .into()
        })
    }

    /// Yanks a crate from crates.io.
    pub(crate) async fn yank_crate(&self, krate: &str, version: &str) -> anyhow::Result<()> {
        self.req::<()>(
            reqwest::Method::DELETE,
            &format!("/crates/{krate}/{version}/yank"),
        )
        .await?
        .error_for_status()?;

        Ok(())
    }

    /// Unyanks a crate from crates.io.
    pub(crate) async fn unyank_crate(&self, krate: &str, version: &str) -> anyhow::Result<()> {
        self.req::<()>(
            reqwest::Method::PUT,
            &format!("/crates/{krate}/{version}/unyank"),
        )
        .await?
        .error_for_status()?;

        Ok(())
    }

    /// Perform a request against the crates.io API
    async fn req<T: Serialize>(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let token = self.get_api_token();
        let req = self
            .client
            .request(method, format!("{CRATES_IO_BASE_URL}{path}"))
            .bearer_auth(token.expose_secret());

        Ok(req.send().await?)
    }
}
