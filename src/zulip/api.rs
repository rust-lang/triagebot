use crate::zulip::client::ZulipClient;
use std::collections::HashMap;

/// A collection of Zulip users, as returned from '/users'
#[derive(serde::Deserialize)]
pub(crate) struct ZulipUsers {
    pub(crate) members: Vec<ZulipUser>,
}

#[derive(Clone, serde::Deserialize, Debug, PartialEq, Eq)]
pub(crate) struct ProfileValue {
    pub(crate) value: String,
}

/// A single Zulip user
#[derive(Clone, serde::Deserialize, Debug, PartialEq, Eq)]
pub(crate) struct ZulipUser {
    pub(crate) user_id: u64,
    #[serde(rename = "full_name")]
    pub(crate) name: String,
    pub(crate) email: String,
    #[serde(default)]
    pub(crate) profile_data: HashMap<String, ProfileValue>,
}

impl ZulipUser {
    // The custom profile field ID for GitHub profiles on the Rust Zulip
    // is 3873. This is likely not portable across different Zulip instance,
    // but we assume that triagebot will only be used on this Zulip instance anyway.
    pub(crate) fn get_github_username(&self) -> Option<&str> {
        self.profile_data.get("3873").map(|v| v.value.as_str())
    }
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct MessageApiResponse {
    #[serde(rename = "id")]
    pub(crate) message_id: u64,
}

#[derive(Copy, Clone, serde::Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub(crate) enum Recipient<'a> {
    Stream {
        #[serde(rename = "to")]
        id: u64,
        topic: &'a str,
    },
    Direct {
        #[serde(skip)]
        id: u64,
        #[serde(rename = "to")]
        email: &'a str,
    },
}

impl Recipient<'_> {
    pub fn narrow(&self) -> String {
        use std::fmt::Write;

        match self {
            Recipient::Stream { id, topic } => {
                // See
                // https://github.com/zulip/zulip/blob/46247623fc279/zerver/lib/url_encoding.py#L9
                // ALWAYS_SAFE without `.` from
                // https://github.com/python/cpython/blob/113e2b0a07c/Lib/urllib/parse.py#L772-L775
                //
                // ALWAYS_SAFE doesn't contain `.` because Zulip actually encodes them to be able
                // to use `.` instead of `%` in the encoded strings
                const ALWAYS_SAFE: &str =
                    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-~";

                let mut encoded_topic = String::new();
                for ch in topic.bytes() {
                    if !(ALWAYS_SAFE.contains(ch as char)) {
                        write!(encoded_topic, ".{:02X}", ch).unwrap();
                    } else {
                        encoded_topic.push(ch as char);
                    }
                }
                format!("stream/{}-xxx/topic/{}", id, encoded_topic)
            }
            Recipient::Direct { id, .. } => format!("pm-with/{}-xxx", id),
        }
    }

    pub fn url(&self, zulip: &ZulipClient) -> String {
        format!("{}/#narrow/{}", zulip.instance_url(), self.narrow())
    }
}

#[cfg(test)]
fn check_encode(topic: &str, expected: &str) {
    const PREFIX: &str = "stream/0-xxx/topic/";
    let computed = Recipient::Stream { id: 0, topic }.narrow();
    assert_eq!(&computed[..PREFIX.len()], PREFIX);
    assert_eq!(&computed[PREFIX.len()..], expected);
}

#[test]
fn test_encode() {
    check_encode("some text with spaces", "some.20text.20with.20spaces");
    check_encode(
        " !\"#$%&'()*+,-./",
        ".20.21.22.23.24.25.26.27.28.29.2A.2B.2C-.2E.2F",
    );
    check_encode("0123456789:;<=>?", "0123456789.3A.3B.3C.3D.3E.3F");
    check_encode(
        "@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_",
        ".40ABCDEFGHIJKLMNOPQRSTUVWXYZ.5B.5C.5D.5E_",
    );
    check_encode(
        "`abcdefghijklmnopqrstuvwxyz{|}~",
        ".60abcdefghijklmnopqrstuvwxyz.7B.7C.7D~.7F",
    );
    check_encode("áé…", ".C3.A1.C3.A9.E2.80.A6");
}
