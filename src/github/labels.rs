use std::{fmt, sync::LazyLock};

use itertools::Itertools;
use regex::Regex;

static EMOJI_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{Emoji}\p{Emoji_Presentation}]").unwrap());

pub(crate) fn normalize_and_match_labels(
    available_labels: &[&str],
    requested_labels: &[&str],
) -> anyhow::Result<Vec<String>> {
    let normalize = |s: &str| EMOJI_REGEX.replace_all(s, "").trim().to_lowercase();

    let mut found_labels = Vec::<String>::with_capacity(requested_labels.len());
    let mut unknown_labels = Vec::new();

    for requested_label in requested_labels {
        // First look for an exact match
        if let Some(found) = available_labels.iter().find(|l| **l == *requested_label) {
            found_labels.push((*found).into());
            continue;
        }

        // Try normalizing requested label (remove emoji, case insensitive, trim whitespace)
        let normalized_requested: String = normalize(requested_label);

        // Find matching labels by normalized name
        let found = available_labels
            .iter()
            .filter(|l| normalize(l) == normalized_requested)
            .collect::<Vec<_>>();

        match found[..] {
            [] => {
                unknown_labels.push(requested_label);
            }
            [label] => {
                found_labels.push((*label).into());
            }
            [..] => {
                return Err(AmbiguousLabelMatch {
                    requested_label: requested_label.to_string(),
                    labels: found.into_iter().map(|l| (*l).into()).collect(),
                }
                .into());
            }
        };
    }

    if !unknown_labels.is_empty() {
        return Err(UnknownLabels {
            labels: unknown_labels.iter().map(|s| s.to_string()).collect(),
        }
        .into());
    }

    Ok(found_labels)
}

#[derive(Debug)]
pub(crate) struct UnknownLabels {
    labels: Vec<String>,
}

// NOTE: This is used to post the Github comment; make sure it's valid markdown.
impl fmt::Display for UnknownLabels {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Unknown labels: {}", &self.labels.join(", "))
    }
}

impl std::error::Error for UnknownLabels {}

#[derive(Debug)]
pub(crate) struct AmbiguousLabelMatch {
    pub requested_label: String,
    pub labels: Vec<String>,
}

// NOTE: This is used to post the Github comment; make sure it's valid markdown.
impl fmt::Display for AmbiguousLabelMatch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Unsure which label to use for `{}` - could be one of: {}",
            self.requested_label,
            self.labels.iter().map(|l| format!("`{}`", l)).join(", ")
        )
    }
}

impl std::error::Error for AmbiguousLabelMatch {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_unknown_labels_error() {
        let x = UnknownLabels {
            labels: vec!["A-bootstrap".into(), "xxx".into()],
        };
        assert_eq!(x.to_string(), "Unknown labels: A-bootstrap, xxx");
    }

    #[test]
    fn display_ambiguous_label_error() {
        let x = AmbiguousLabelMatch {
            requested_label: "A-bootstrap".into(),
            labels: vec!["A-bootstrap".into(), "A-bootstrap-2".into()],
        };
        assert_eq!(
            x.to_string(),
            "Unsure which label to use for `A-bootstrap` - could be one of: `A-bootstrap`, `A-bootstrap-2`"
        );
    }

    #[test]
    fn normalize_and_match_labels_happy_path() {
        let available_labels = vec!["A-bootstrap 😺", "B-foo 👾", "C-bar", "C-bar 😦"];
        let requested_labels = vec!["A-bootstrap", "B-foo", "C-bar"];

        let result = normalize_and_match_labels(&available_labels, &requested_labels);

        assert!(result.is_ok());
        let found_labels = result.unwrap();
        assert_eq!(found_labels.len(), 3);
        assert_eq!(found_labels[0], "A-bootstrap 😺");
        assert_eq!(found_labels[1], "B-foo 👾");
        assert_eq!(found_labels[2], "C-bar");
    }

    #[test]
    fn normalize_and_match_labels_no_match() {
        let available_labels = vec!["A-bootstrap", "B-foo"];
        let requested_labels = vec!["A-bootstrap", "C-bar"];

        let result = normalize_and_match_labels(&available_labels, &requested_labels);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is::<UnknownLabels>());
        let unknown = err.downcast::<UnknownLabels>().unwrap();
        assert_eq!(unknown.labels, vec!["C-bar"]);
    }

    #[test]
    fn normalize_and_match_labels_ambiguous_match() {
        let available_labels = vec!["A-bootstrap 😺", "A-bootstrap 👾"];
        let requested_labels = vec!["A-bootstrap"];

        let result = normalize_and_match_labels(&available_labels, &requested_labels);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is::<AmbiguousLabelMatch>());
        let ambiguous = err.downcast::<AmbiguousLabelMatch>().unwrap();
        assert_eq!(ambiguous.requested_label, "A-bootstrap");
        assert_eq!(ambiguous.labels, vec!["A-bootstrap 😺", "A-bootstrap 👾"]);
    }
}
