mod rustc;

use comrak::Arena;
use std::collections::HashMap;

#[derive(Copy, Clone, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ChangelogFormat {
    Rustc,
}

pub(crate) struct Changelog {
    versions: HashMap<String, String>,
}

impl Changelog {
    pub(crate) fn parse(format: ChangelogFormat, content: &str) -> anyhow::Result<Self> {
        match format {
            ChangelogFormat::Rustc => rustc::RustcFormat::new(&Arena::new()).parse(content),
        }
    }

    pub(crate) fn version(&self, version: &str) -> Option<&str> {
        self.versions.get(version).map(|s| s.as_str())
    }
}
