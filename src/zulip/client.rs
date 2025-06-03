use crate::zulip::api::{MessageApiResponse, ZulipUser, ZulipUsers};
use crate::zulip::Recipient;
use anyhow::Context;
use reqwest::{Client, Method, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use std::env;
use std::sync::OnceLock;

#[derive(Clone)]
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
            .make_request(Method::GET, "users?include_custom_profile_fields=true")
            .send()
            .await?;
        deserialize_response::<ZulipUsers>(resp)
            .await
            .map(|users| users.members)
    }

    pub(crate) async fn send_message<'a>(
        &self,
        recipient: Recipient<'a>,
        content: &'a str,
    ) -> anyhow::Result<MessageApiResponse> {
        #[derive(serde::Serialize)]
        struct SerializedApi<'a> {
            #[serde(rename = "type")]
            type_: &'static str,
            to: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            topic: Option<&'a str>,
            content: &'a str,
        }

        let response = self
            .make_request(Method::POST, "messages")
            .form(&SerializedApi {
                type_: match recipient {
                    Recipient::Stream { .. } => "stream",
                    Recipient::Private { .. } => "private",
                },
                to: match recipient {
                    Recipient::Stream { id, .. } => id.to_string(),
                    Recipient::Private { email, .. } => email.to_string(),
                },
                topic: match recipient {
                    Recipient::Stream { topic, .. } => Some(topic),
                    Recipient::Private { .. } => None,
                },
                content,
            })
            .send()
            .await
            .context("fail sending Zulip message")?;

        deserialize_response::<MessageApiResponse>(response).await
    }

    pub(crate) async fn update_message<'a>(
        &self,
        message_id: u64,
        topic: Option<&'a str>,
        propagate_mode: Option<&'a str>,
        content: Option<&'a str>,
    ) -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct SerializedApi<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            topic: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            propagate_mode: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            content: Option<&'a str>,
        }

        let resp = self
            .make_request(Method::PATCH, &format!("messages/{message_id}"))
            .form(&SerializedApi {
                topic,
                propagate_mode,
                content,
            })
            .send()
            .await
            .context("failed to send Zulip API Update Message")?;

        let status = resp.status();

        if !status.is_success() {
            let body = resp
                .text()
                .await
                .context("fail receiving Zulip API response (when updating the message)")?;

            anyhow::bail!(body)
        }

        Ok(())
    }

    pub(crate) async fn add_reaction(
        &self,
        message_id: u64,
        emoji_name: &str,
    ) -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct AddReaction<'a> {
            message_id: u64,
            emoji_name: &'a str,
        }

        let resp = self
            .make_request(Method::POST, &format!("messages/{message_id}/reactions"))
            .form(&AddReaction {
                message_id,
                emoji_name,
            })
            .send()
            .await
            .context("failed to add reaction to a Zulip message")?;

        let status = resp.status();

        if !status.is_success() {
            let body = resp
                .text()
                .await
                .context("fail receiving Zulip API response (when adding a reaction)")?;

            anyhow::bail!(body)
        }

        Ok(())
    }

    fn make_request(&self, method: Method, url: &str) -> RequestBuilder {
        let api_token = self.get_api_token();
        self.client
            .request(method, &format!("{}/api/v1/{url}", self.instance_url))
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
