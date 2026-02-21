use regex::Regex;
use std::collections::HashMap;

use crate::github::Comment;

#[derive(Debug)]
pub enum Selection<'a, T: ?Sized> {
    All,
    One(&'a T),
    Except(&'a T),
}

/// Deserialize as an optional string
pub(crate) fn opt_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    use serde::de::Deserialize;
    match <Option<String>>::deserialize(deserializer) {
        Ok(v) => Ok(v.unwrap_or_default()),
        Err(e) => Err(e),
    }
}

/// Quote a markdown input as a reply
pub(crate) fn quote_reply(markdown: &str) -> String {
    if markdown.is_empty() {
        String::from("*No content*")
    } else {
        format!("\n\t> {}", markdown.replace('\n', "\n\t> "))
    }
}

/// Return open concerns filed in an issue under MCP/RFC process
/// Concerns are marked by `@rustbot concern` and `@rustbot resolve`
pub(crate) fn find_open_concerns(comments: Vec<Comment>) -> Option<Vec<(String, String)>> {
    let re_concern_raise =
        Regex::new(r"@rustbot concern (?P<concern_title>.*)").expect("Invalid regexp");
    let re_concern_solve =
        Regex::new(r"@rustbot resolve (?P<concern_title>.*)").expect("Invalid regexp");
    let mut raised: HashMap<String, String> = HashMap::new();
    let mut solved: HashMap<String, String> = HashMap::new();

    for comment in comments {
        // Parse the comment and look for text markers to raise or resolve concerns
        let comment_lines = comment.body.lines();
        for line in comment_lines {
            let r: Vec<&str> = re_concern_raise
                .captures_iter(line)
                .map(|caps| caps.name("concern_title").map(|f| f.as_str()).unwrap_or(""))
                .collect();
            let s: Vec<&str> = re_concern_solve
                .captures_iter(line)
                .map(|caps| caps.name("concern_title").map(|f| f.as_str()).unwrap_or(""))
                .collect();

            // pick the first match only
            if !r.is_empty() {
                let x = r[0].replace("@rustbot concern", "");
                raised.insert(x.trim().to_string(), comment.html_url.to_string());
            }
            if !s.is_empty() {
                let x = s[0].replace("@rustbot resolve", "");
                solved.insert(x.trim().to_string(), comment.html_url.to_string());
            }
        }
    }

    // remove solved concerns and return the rest
    let unresolved_concerns = raised
        .iter()
        .filter_map(|(title, comment_url)| {
            if solved.contains_key(title) {
                None
            } else {
                Some((title.to_string(), comment_url.to_string()))
            }
        })
        .collect();

    Some(unresolved_concerns)
}
