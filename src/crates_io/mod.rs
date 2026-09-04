use reqwest::Client;
use reqwest::header;
use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::env;
use std::sync::OnceLock;
use tracing::instrument;
use url::Url;

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
    #[instrument(level = "info", skip(self))]
    pub async fn yank_crate(&self, krate: &str, version: &semver::Version) -> anyhow::Result<()> {
        self.req::<()>(
            reqwest::Method::DELETE,
            self.url(&["crates", krate, &version.to_string(), "yank"]),
        )
        .await?
        .error_for_status()?;

        Ok(())
    }

    /// Unyanks a crate from crates.io.
    #[instrument(level = "info", skip(self))]
    pub async fn unyank_crate(&self, krate: &str, version: &semver::Version) -> anyhow::Result<()> {
        self.req::<()>(
            reqwest::Method::PUT,
            self.url(&["crates", krate, &version.to_string(), "unyank"]),
        )
        .await?
        .error_for_status()?;

        Ok(())
    }

    /// Performs a request against the crates.io API
    async fn req<T: Serialize>(
        &self,
        method: reqwest::Method,
        url: Url,
    ) -> anyhow::Result<reqwest::Response> {
        let token = self.get_api_token();
        let req = self
            .client
            .request(method, url)
            .bearer_auth(token.expose_secret());

        Ok(req.send().await?)
    }

    /// We construct the URL from individual segments, to avoid possible "URL injection" by using
    /// segments like "../<path>", which could overwrite other segments of the URL.
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = Url::parse(CRATES_IO_BASE_URL).unwrap();
        {
            let mut url_segments = url.path_segments_mut().unwrap();
            for segment in segments {
                url_segments.push(segment);
            }
        }
        url
    }
}
