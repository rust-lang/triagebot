//! Purpose: When opening a PR, or pushing new changes, check for github mentions
//! in commits and notify the user of our no-mentions in commits policy.

use crate::{config::NoMentionsConfig, github::GithubCommit};

pub(super) fn mentions_in_commits(
    pr_title: &str,
    conf: &NoMentionsConfig,
    commits: &[GithubCommit],
) -> Option<String> {
    if conf.exclude_titles.iter().any(|s| pr_title.contains(s)) {
        return None;
    }

    let mentions_commits = commits
        .into_iter()
        .filter(|c| {
            let mentions = parser::get_mentions(&c.commit.message);
            !mentions.is_empty() && mentions.iter().any(|m| *m != "rustbot")
        })
        .map(|c| format!("- {}\n", c.sha))
        .collect::<String>();

    if mentions_commits.is_empty() {
        None
    } else {
        Some(format!(
            r"There are username mentions (such as `@user`) in the commit messages of the following commits.
*Please remove the mentions to avoid spamming these users.*
{mentions_commits}",
        ))
    }
}

#[test]
fn test_mentions_in_commits() {
    use super::dummy_commit_from_body;

    let mut commits = vec![dummy_commit_from_body(
        "d1992a392617dfb10518c3e56446b6c9efae38b0",
        "This is simple without mentions!",
    )];

    let default_conf = NoMentionsConfig {
        exclude_titles: vec![],
    };

    assert_eq!(
        mentions_in_commits("any title", &default_conf, &commits),
        None
    );

    commits.push(dummy_commit_from_body(
        "10b96a74c484cae79164cbbcdfcd412109e0e4cf",
        r"This is a body with a sign-off and co-author
Signed-off-by: Foo Bar <foobar123@example.com>
Co-authored-by: Baz Qux <bazqux@example.com>",
    ));

    assert_eq!(
        mentions_in_commits("any title", &default_conf, &commits,),
        None
    );

    commits.push(dummy_commit_from_body(
        "6565ffdd8af4ca0ec7c8faceee59c582edcd83b2",
        "This is a body that only mentions @rustbot for a command!",
    ));

    assert_eq!(
        mentions_in_commits("any title", &default_conf, &commits),
        None
    );

    commits.push(dummy_commit_from_body(
        "6565ffdd8af4ca0ec7c8faceee59c582edcd83b2",
        "This is a body that mentions @rustbot for a command! And then a user @mention",
    ));

    assert_eq!(
        mentions_in_commits("any title", &default_conf, &commits, ),
        Some(
            r"There are username mentions (such as `@user`) in the commit messages of the following commits.
*Please remove the mentions to avoid spamming these users.*
- 6565ffdd8af4ca0ec7c8faceee59c582edcd83b2
".to_string()
        )
    );

    let _ = commits.pop(); // Remove that @rustbot & @mention case

    commits.push(dummy_commit_from_body(
        "d7daa17bc97df9377640b0d33cbd0bbeed703c3a",
        "This is a body with a @mention!",
    ));

    assert_eq!(
        mentions_in_commits(
            "exclude this pull from checking ",
            &NoMentionsConfig {
                exclude_titles: vec![String::from("exclude this")]
            },
            &commits
        ),
        None
    );
}
