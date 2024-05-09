use crate::github;
use crate::{
    config::DecisionConfig, db::issue_decision_state::*, github::Event, handlers::Context,
    interactions::ErrorComment,
};
use anyhow::bail;
use anyhow::Context as Ctx;
use chrono::{DateTime, Duration, Utc};
use parser::command::decision::Resolution;
use parser::command::decision::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const _DECISION_PROCESS_JOB_NAME: &str = "decision_process_action";

#[derive(Serialize, Deserialize)]
pub struct DecisionProcessActionMetadata {
    pub message: String,
    pub get_issue_url: String,
    pub status: Resolution,
}

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &DecisionConfig,
    event: &Event,
    cmd: DecisionCommand,
) -> anyhow::Result<()> {
    let db = ctx.db.get().await;

    let DecisionCommand {
        resolution,
        reversibility,
        team: team_name,
    } = cmd;

    let issue = event.issue().unwrap();
    let user = event.user();

    match get_issue_decision_state(&db, &issue.number).await {
        Ok(_state) => {
            // TO DO
            let cmnt = ErrorComment::new(
                &issue,
                "We don't support having more than one vote yet. Coming soon :)",
            );
            cmnt.post(&ctx.github).await?;

            Ok(())
        }
        _ => {
            let is_team_member = user.is_team_member(&ctx.github).await.unwrap_or(false);
            if !is_team_member {
                let cmnt = ErrorComment::new(
                    &issue,
                    "Only team members can be part of the decision process.",
                );
                cmnt.post(&ctx.github).await?;

                return Ok(());
            }

            match team_name {
                None => {
                    let cmnt = ErrorComment::new(
                        &issue,
                        "In the first vote, is necessary to specify the team name that will be involved in the decision process.",
                    );
                    cmnt.post(&ctx.github).await?;

                    Ok(())
                }
                Some(team_name) => {
                    match github::get_team(&ctx.github, &team_name).await {
                        Ok(Some(team)) => {
                            let start_date: DateTime<Utc> = chrono::Utc::now().into();
                            let end_date: DateTime<Utc> =
                                start_date.checked_add_signed(Duration::days(10)).unwrap();

                            let mut current: BTreeMap<String, Option<UserStatus>> = BTreeMap::new();
                            let mut history: BTreeMap<String, Vec<UserStatus>> = BTreeMap::new();

                            // Add team members to current and history
                            for member in team.members {
                                current.insert(member.github.clone(), None);
                                history.insert(member.github.clone(), Vec::new());
                            }

                            // Add issue user to current and history
                            current.insert(
                                user.login.clone(),
                                Some(UserStatus {
                                    comment_id: event.html_url().unwrap().to_string(),
                                    text: event.comment_body().unwrap().to_string(),
                                    reversibility: reversibility,
                                    resolution: resolution,
                                }),
                            );
                            history.insert(user.login.clone(), Vec::new());

                            // Initialize issue decision state
                            insert_issue_decision_state(
                                &db,
                                &issue.number,
                                &user.login,
                                &start_date,
                                &end_date,
                                &current,
                                &history,
                                &reversibility,
                                &resolution,
                            )
                            .await?;

                            // TO DO -- Do not insert this job until we support more votes
                            // let metadata = serde_json::value::to_value(DecisionProcessActionMetadata {
                            //     message: "some message".to_string(),
                            //     get_issue_url: format!("{}/issues/{}", issue.repository().url(), issue.number),
                            //     status: resolution,
                            // })
                            // .unwrap();
                            // insert_job(
                            //     &db,
                            //     &DECISION_PROCESS_JOB_NAME.to_string(),
                            //     &end_date,
                            //     &metadata,
                            // )
                            // .await?;

                            let comment = build_status_comment(&history, &current)?;
                            issue
                                .post_comment(&ctx.github, &comment)
                                .await
                                .context("merge vote comment")?;

                            Ok(())
                        }
                        _ => {
                            let cmnt =
                                ErrorComment::new(&issue, "Failed to resolve to a known team.");
                            cmnt.post(&ctx.github).await?;

                            Ok(())
                        }
                    }
                }
            }
        }
    }
}

fn build_status_comment(
    history: &BTreeMap<String, Vec<UserStatus>>,
    current: &BTreeMap<String, Option<UserStatus>>,
) -> anyhow::Result<String> {
    let mut comment = "| Team member | State |\n|-------------|-------|".to_owned();
    for (user, status) in current {
        let mut user_statuses = format!("\n| @{} |", user);

        // previous stasuses
        match history.get(user) {
            Some(statuses) => {
                for status in statuses {
                    let status_item =
                        format!(" [~~{}~~]({}) ", status.resolution, status.comment_id);
                    user_statuses.push_str(&status_item);
                }
            }
            None => bail!("user {} not present in history statuses list", user),
        }

        // current status
        let user_resolution = match status {
            Some(status) => format!("[**{}**]({})", status.resolution, status.comment_id),
            _ => "".to_string(),
        };

        let status_item = format!(" {} |", user_resolution);
        user_statuses.push_str(&status_item);

        comment.push_str(&user_statuses);
    }

    Ok(comment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use factori::{create, factori};

    factori!(UserStatus, {
        default {
            comment_id = "https://some-comment-id-for-merge.com".to_string(),
            text = "this is my argument for making this decision".to_string(),
            reversibility = Reversibility::Reversible,
            resolution = Resolution::Merge
        }

        mixin hold {
            comment_id = "https://some-comment-id-for-hold.com".to_string(),
            resolution = Resolution::Hold
        }
    });

    #[test]
    fn test_successfuly_build_comment() {
        let mut history: BTreeMap<String, Vec<UserStatus>> = BTreeMap::new();
        let mut current_statuses: BTreeMap<String, Option<UserStatus>> = BTreeMap::new();

        // user 1
        let mut user_1_statuses: Vec<UserStatus> = Vec::new();
        user_1_statuses.push(create!(UserStatus));
        user_1_statuses.push(create!(UserStatus, :hold));

        history.insert("Niklaus".to_string(), user_1_statuses);

        current_statuses.insert("Niklaus".to_string(), Some(create!(UserStatus)));

        // user 2
        let mut user_2_statuses: Vec<UserStatus> = Vec::new();
        user_2_statuses.push(create!(UserStatus, :hold));
        user_2_statuses.push(create!(UserStatus));

        history.insert("Barbara".to_string(), user_2_statuses);

        current_statuses.insert("Barbara".to_string(), Some(create!(UserStatus)));

        let build_result = build_status_comment(&history, &current_statuses)
            .expect("it shouldn't fail building the message");
        let expected_comment = "| Team member | State |\n\
        |-------------|-------|\n\
        | @Barbara | [~~hold~~](https://some-comment-id-for-hold.com)  [~~merge~~](https://some-comment-id-for-merge.com)  [**merge**](https://some-comment-id-for-merge.com) |\n\
        | @Niklaus | [~~merge~~](https://some-comment-id-for-merge.com)  [~~hold~~](https://some-comment-id-for-hold.com)  [**merge**](https://some-comment-id-for-merge.com) |"
            .to_string();

        assert_eq!(build_result, expected_comment);
    }

    #[test]
    fn test_successfuly_build_comment_user_no_votes() {
        let mut history: BTreeMap<String, Vec<UserStatus>> = BTreeMap::new();
        let mut current_statuses: BTreeMap<String, Option<UserStatus>> = BTreeMap::new();

        // user 1
        let mut user_1_statuses: Vec<UserStatus> = Vec::new();
        user_1_statuses.push(create!(UserStatus));
        user_1_statuses.push(create!(UserStatus, :hold));

        history.insert("Niklaus".to_string(), user_1_statuses);

        current_statuses.insert("Niklaus".to_string(), Some(create!(UserStatus)));

        // user 2
        let mut user_2_statuses: Vec<UserStatus> = Vec::new();
        user_2_statuses.push(create!(UserStatus, :hold));
        user_2_statuses.push(create!(UserStatus));

        history.insert("Barbara".to_string(), user_2_statuses);

        current_statuses.insert("Barbara".to_string(), Some(create!(UserStatus)));

        // user 3
        history.insert("Tom".to_string(), Vec::new());

        current_statuses.insert("Tom".to_string(), None);

        let build_result = build_status_comment(&history, &current_statuses)
            .expect("it shouldn't fail building the message");
        let expected_comment = "| Team member | State |\n\
        |-------------|-------|\n\
        | @Barbara | [~~hold~~](https://some-comment-id-for-hold.com)  [~~merge~~](https://some-comment-id-for-merge.com)  [**merge**](https://some-comment-id-for-merge.com) |\n\
        | @Niklaus | [~~merge~~](https://some-comment-id-for-merge.com)  [~~hold~~](https://some-comment-id-for-hold.com)  [**merge**](https://some-comment-id-for-merge.com) |\n\
        | @Tom |  |"
            .to_string();

        assert_eq!(build_result, expected_comment);
    }

    #[test]
    fn test_build_comment_inconsistent_users() {
        let mut history: BTreeMap<String, Vec<UserStatus>> = BTreeMap::new();
        let mut current_statuses: BTreeMap<String, Option<UserStatus>> = BTreeMap::new();

        // user 1
        let mut user_1_statuses: Vec<UserStatus> = Vec::new();
        user_1_statuses.push(create!(UserStatus));
        user_1_statuses.push(create!(UserStatus, :hold));

        history.insert("Niklaus".to_string(), user_1_statuses);

        current_statuses.insert("Niklaus".to_string(), Some(create!(UserStatus)));

        // user 2
        let mut user_2_statuses: Vec<UserStatus> = Vec::new();
        user_2_statuses.push(create!(UserStatus, :hold));
        user_2_statuses.push(create!(UserStatus));

        history.insert("Barbara".to_string(), user_2_statuses);

        current_statuses.insert("Martin".to_string(), Some(create!(UserStatus)));

        let build_result = build_status_comment(&history, &current_statuses);
        assert_eq!(
            format!("{}", build_result.unwrap_err()),
            "user Martin not present in history statuses list"
        );
    }

    #[test]
    fn test_successfuly_build_comment_no_history() {
        let mut history: BTreeMap<String, Vec<UserStatus>> = BTreeMap::new();
        let mut current_statuses: BTreeMap<String, Option<UserStatus>> = BTreeMap::new();

        // user 1
        let mut user_1_statuses: Vec<UserStatus> = Vec::new();
        user_1_statuses.push(create!(UserStatus));
        user_1_statuses.push(create!(UserStatus, :hold));

        current_statuses.insert("Niklaus".to_string(), Some(create!(UserStatus)));
        history.insert("Niklaus".to_string(), Vec::new());

        // user 2
        let mut user_2_statuses: Vec<UserStatus> = Vec::new();
        user_2_statuses.push(create!(UserStatus, :hold));
        user_2_statuses.push(create!(UserStatus));

        current_statuses.insert("Barbara".to_string(), Some(create!(UserStatus)));
        history.insert("Barbara".to_string(), Vec::new());

        let build_result = build_status_comment(&history, &current_statuses)
            .expect("it shouldn't fail building the message");
        let expected_comment = "| Team member | State |\n\
        |-------------|-------|\n\
        | @Barbara | [**merge**](https://some-comment-id-for-merge.com) |\n\
        | @Niklaus | [**merge**](https://some-comment-id-for-merge.com) |\
        "
        .to_string();

        assert_eq!(build_result, expected_comment);
    }
}
