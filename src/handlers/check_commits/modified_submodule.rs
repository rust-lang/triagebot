use crate::github::FileDiff;

const SUBMODULE_WARNING_MSG: &str = "These commits modify **submodules**.";

/// Returns a message if the PR modifies a git submodule.
pub(super) fn modifies_submodule(diff: &[FileDiff]) -> Option<String> {
    let re = regex::Regex::new(r"\+Subproject\scommit\s").unwrap();
    if diff.iter().any(|fd| re.is_match(&fd.diff)) {
        Some(SUBMODULE_WARNING_MSG.to_string())
    } else {
        None
    }
}
