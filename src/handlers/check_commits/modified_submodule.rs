use crate::github::FileDiff;

const SUBMODULE_WARNING_MSG: &str = "Some commits in this PR modify **submodules**.";

/// Returns a message if the PR modifies a git submodule.
pub(super) fn modifies_submodule(diff: &[FileDiff]) -> Option<String> {
    let re = regex::Regex::new(r"\+Subproject\scommit\s").unwrap();
    if diff.iter().any(|fd| re.is_match(&fd.patch)) {
        Some(SUBMODULE_WARNING_MSG.to_string())
    } else {
        None
    }
}
