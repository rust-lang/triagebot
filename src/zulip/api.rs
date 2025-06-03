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
