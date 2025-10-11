//! Purpose: Allow any user to modify issue labels on GitHub via comments.
//!
//! Labels are checked against the labels in the project; the bot does not support creating new
//! labels.
//!
//! Parsing is done in the `parser::command::relabel` module.
//!
//! If the command was successful, there will be no feedback beyond the label change to reduce
//! notification noise.

use std::collections::BTreeSet;

use crate::github::Label;
use crate::team_data::TeamClient;
use crate::{
    config::RelabelConfig,
    github::UnknownLabels,
    github::{self, Event},
    handlers::Context,
};
use parser::command::relabel::{LabelDelta, RelabelCommand};

pub(super) async fn handle_command(
    ctx: &Context,
    config: &RelabelConfig,
    event: &Event,
    input: RelabelCommand,
) -> anyhow::Result<()> {
    let Some(issue) = event.issue() else {
        return user_error!("Can only add and remove labels on an issue");
    };

    // Check label authorization for the current user
    for delta in &input.0 {
        let name = delta.label() as &str;
        let err = match check_filter(name, config, is_member(event.user(), &ctx.team).await) {
            Ok(CheckFilterResult::Allow) => None,
            Ok(CheckFilterResult::Deny) => {
                Some(format!("Label {name} can only be set by Rust team members"))
            }
            Ok(CheckFilterResult::DenyUnknown) => Some(format!(
                "Label {name} can only be set by Rust team members;\
                 we were unable to check if you are a team member."
            )),
            Err(err) => Some(err),
        };
        if let Some(err) = err {
            // bail-out and inform the user why
            return user_error!(err);
        }
    }

    // Compute the labels to add and remove
    let (to_add, to_remove) = compute_label_deltas(&input.0);

    // Add labels
    if let Err(e) = issue.add_labels(&ctx.github, to_add.clone()).await {
        tracing::error!(
            "failed to add {to_add:?} from issue {issue}: {e:?}",
            issue = issue.global_id(),
        );
        if let Some(err @ UnknownLabels { .. }) = e.downcast_ref() {
            issue.post_comment(&ctx.github, &err.to_string()).await?;
        }

        return Err(e);
    }

    // Remove labels
    if let Err(e) = issue.remove_labels(&ctx.github, to_remove.clone()).await {
        tracing::error!(
            "failed to remove {to_remove:?} from issue {issue}: {e:?}",
            issue = issue.global_id(),
        );
        return Err(e);
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum TeamMembership {
    Member,
    Outsider,
    Unknown,
}

async fn is_member(user: &github::User, client: &TeamClient) -> TeamMembership {
    match user.is_team_member(client).await {
        Ok(true) => TeamMembership::Member,
        Ok(false) => TeamMembership::Outsider,
        Err(err) => {
            eprintln!("failed to check team membership: {err:?}");
            TeamMembership::Unknown
        }
    }
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
enum CheckFilterResult {
    Allow,
    Deny,
    DenyUnknown,
}

fn check_filter(
    label: &str,
    config: &RelabelConfig,
    is_member: TeamMembership,
) -> Result<CheckFilterResult, String> {
    if is_member == TeamMembership::Member {
        return Ok(CheckFilterResult::Allow);
    }
    let mut matched = false;
    for pattern in &config.allow_unauthenticated {
        match match_pattern(pattern, label) {
            Ok(MatchPatternResult::Allow) => matched = true,
            Ok(MatchPatternResult::Deny) => {
                // An explicit deny overrides any allowed pattern
                matched = false;
                break;
            }
            Ok(MatchPatternResult::NoMatch) => {}
            Err(err) => {
                eprintln!("failed to match pattern {pattern}: {err}");
                return Err(format!("failed to match pattern {pattern}"));
            }
        }
    }
    if matched {
        Ok(CheckFilterResult::Allow)
    } else if is_member == TeamMembership::Outsider {
        Ok(CheckFilterResult::Deny)
    } else {
        Ok(CheckFilterResult::DenyUnknown)
    }
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
enum MatchPatternResult {
    Allow,
    Deny,
    NoMatch,
}

fn match_pattern(pattern: &str, label: &str) -> anyhow::Result<MatchPatternResult> {
    let (pattern, inverse) = if let Some(pat) = pattern.strip_prefix('!') {
        (pat, true)
    } else {
        (pattern, false)
    };

    let glob = glob::Pattern::new(pattern)?;
    let matchopts = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };

    Ok(match (glob.matches_with(label, matchopts), inverse) {
        (true, false) => MatchPatternResult::Allow,
        (true, true) => MatchPatternResult::Deny,
        (false, _) => MatchPatternResult::NoMatch,
    })
}

fn compute_label_deltas(deltas: &[LabelDelta]) -> (Vec<Label>, Vec<Label>) {
    let mut add = BTreeSet::new();
    let mut remove = BTreeSet::new();

    for delta in deltas {
        match delta {
            LabelDelta::Add(label) => {
                let label = Label {
                    name: label.to_string(),
                };
                if !remove.remove(&label) {
                    add.insert(label);
                }
            }
            LabelDelta::Remove(label) => {
                let label = Label {
                    name: label.to_string(),
                };
                if !add.remove(&label) {
                    remove.insert(label);
                }
            }
        }
    }

    (add.into_iter().collect(), remove.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use parser::command::relabel::{Label, LabelDelta};

    use super::{
        CheckFilterResult, MatchPatternResult, TeamMembership, check_filter, compute_label_deltas,
        match_pattern,
    };
    use crate::config::RelabelConfig;

    #[test]
    fn test_match_pattern() -> anyhow::Result<()> {
        assert_eq!(
            match_pattern("I-*", "I-nominated")?,
            MatchPatternResult::Allow
        );
        assert_eq!(
            match_pattern("i-*", "I-nominated")?,
            MatchPatternResult::Allow
        );
        assert_eq!(
            match_pattern("!I-no*", "I-nominated")?,
            MatchPatternResult::Deny
        );
        assert_eq!(
            match_pattern("I-*", "T-infra")?,
            MatchPatternResult::NoMatch
        );
        assert_eq!(
            match_pattern("!I-no*", "T-infra")?,
            MatchPatternResult::NoMatch
        );
        Ok(())
    }

    #[test]
    fn test_check_filter() -> anyhow::Result<()> {
        macro_rules! t {
            ($($member:ident { $($label:expr => $res:ident,)* })*) => {
                let config = RelabelConfig {
                    allow_unauthenticated: vec!["T-*".into(), "I-*".into(), "!I-*nominated".into()],
                };
                $($(assert_eq!(
                    check_filter($label, &config, TeamMembership::$member),
                    Ok(CheckFilterResult::$res)
                );)*)*
            }
        }
        t! {
            Member {
                "T-release" => Allow,
                "I-slow" => Allow,
                "I-lang-nominated" => Allow,
                "I-nominated" => Allow,
                "A-spurious" => Allow,
            }
            Outsider {
                "T-release" => Allow,
                "I-slow" => Allow,
                "I-lang-nominated" => Deny,
                "I-nominated" => Deny,
                "A-spurious" => Deny,
            }
            Unknown {
                "T-release" => Allow,
                "I-slow" => Allow,
                "I-lang-nominated" => DenyUnknown,
                "I-nominated" => DenyUnknown,
                "A-spurious" => DenyUnknown,
            }
        }
        Ok(())
    }

    #[test]
    fn test_compute_label_deltas() {
        use crate::github::Label as GitHubLabel;

        let mut deltas = vec![
            LabelDelta::Add(Label("I-nominated".to_string())),
            LabelDelta::Add(Label("I-nominated".to_string())),
            LabelDelta::Add(Label("I-lang-nominated".to_string())),
            LabelDelta::Add(Label("I-libs-nominated".to_string())),
            LabelDelta::Remove(Label("I-lang-nominated".to_string())),
        ];

        assert_eq!(
            compute_label_deltas(&deltas),
            (
                vec![
                    GitHubLabel {
                        name: "I-libs-nominated".to_string()
                    },
                    GitHubLabel {
                        name: "I-nominated".to_string()
                    },
                ],
                vec![],
            ),
        );

        deltas.push(LabelDelta::Remove(Label("needs-triage".to_string())));
        deltas.push(LabelDelta::Add(Label("I-lang-nominated".to_string())));

        assert_eq!(
            compute_label_deltas(&deltas),
            (
                vec![
                    GitHubLabel {
                        name: "I-lang-nominated".to_string()
                    },
                    GitHubLabel {
                        name: "I-libs-nominated".to_string()
                    },
                    GitHubLabel {
                        name: "I-nominated".to_string()
                    },
                ],
                vec![GitHubLabel {
                    name: "needs-triage".to_string()
                }],
            ),
        );
    }
}
