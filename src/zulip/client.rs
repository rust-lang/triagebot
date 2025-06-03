use crate::zulip::api::{ZulipUser, ZulipUsers};
use anyhow::Context;
use reqwest::{Client, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use std::env;
use std::sync::OnceLock;

pub struct ZulipClient {
    client: Client,
    instance_url: String,
    bot_email: String,
    // The token is loaded lazily, to avoid requiring the API token if Zulip APIs are not
    // actually accessed.
    bot_api_token: OnceLock<String>,
}

impl ZulipClient {
    pub fn new_from_env() -> Self {
        let instance_url =
            env::var("ZULIP_URL").unwrap_or("https://rust-lang.zulipchat.com".into());
        let bot_email =
            env::var("ZULIP_BOT_EMAIL").unwrap_or("triage-rust-lang-bot@zulipchat.com".into());
        Self::new(instance_url, bot_email)
    }

    fn new(instance_url: String, bot_email: String) -> Self {
        let client = Client::new();
        Self {
            client,
            instance_url,
            bot_email,
            bot_api_token: OnceLock::new(),
        }
    }

    // Taken from https://github.com/kobzol/team/blob/0f68ffc8b0d438d88ef4573deb54446d57e1eae6/src/api/zulip.rs#L45
    pub(crate) async fn get_zulip_users(&self) -> anyhow::Result<Vec<ZulipUser>> {
        let resp = self
            .make_request("api/v1/users?include_custom_profile_fields=true")
            .send()
            .await?;
        deserialize_response::<ZulipUsers>(resp)
            .await
            .map(|users| users.members)
    }

    fn make_request(&self, url: &str) -> RequestBuilder {
        let api_token = self.get_api_token();
        self.client
            .get(&format!("{}/{url}", self.instance_url))
            .basic_auth(&self.bot_email, Some(api_token))
    }

    fn get_api_token(&self) -> &str {
        self.bot_api_token
            .get_or_init(|| env::var("ZULIP_API_TOKEN").expect("ZULIP_API_TOKEN is missing"))
            .as_ref()
    }
}

async fn deserialize_response<T>(response: Response) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.context("Zulip API request failed")?;
        Err(anyhow::anyhow!(body))
    } else {
        Ok(response.json::<T>().await.with_context(|| {
            anyhow::anyhow!(
                "Failed to deserialize value of type {}",
                std::any::type_name::<T>()
            )
        })?)
    }
}
