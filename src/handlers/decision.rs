use crate::db::jobs::*;
use crate::github;
use crate::{
    config::DecisionConfig, db::issue_decision_state::*, github::Event, handlers::Context,
    interactions::ErrorComment,
};
use anyhow::bail;
use anyhow::Context as Ctx;
use chrono::{DateTime, Duration, Utc};
use parser::command::decision::Resolution::{Hold, Merge};
use parser::command::decision::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// get state for issue_id from db
    // if no state (first call)
        // initialize state
        // schedule job if necessary
        // send comment to github
        // save state
    // if state
        // apply logic to decide what to do
        // schedule job if necessary
        // send comment to github
        // save state

pub const DECISION_PROCESS_JOB_NAME: &str = "decision_process_action";

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
    } = cmd;

    let issue = event.issue().unwrap();
    let user = event.user();

    let is_team_member = user.is_team_member(&ctx.github).await.unwrap_or(false);

    if !is_team_member {
        let cmnt = ErrorComment::new(
            &issue,
            "Only team members can be part of the decision process.",
        );
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    match get_issue_decision_state(&db, &issue.number).await {
        Ok(_state) => {
            // let name = match disposition {
            //     Hold => "hold".into(),
            //     Custom(name) => name,
            // };

            // let mut current_statuses = state.current_statuses;
            // let mut status_history = state.status_history;

            // if let Some(entry) = current_statuses.get_mut(&user) {
            //     let past = status_history.entry(user).or_insert(Vec::new());

            //     past.push(entry.clone());

            //     *entry = UserStatus::new(name, issue_id, comment_id);
            // } else {
            //     current_statuses.insert(user, UserStatus::new("hold".into(), issue_id, comment_id));
            // }

            // Ok(State {
            //     current_statuses,
            //     status_history,
            //     ..state
            // })
            Ok(())
        }
        _ => {
            match resolution {
                Hold => Ok(()), // change me!
                Merge => {
                    let start_date: DateTime<Utc> = chrono::Utc::now().into();
                    let end_date: DateTime<Utc> =
                        start_date.checked_add_signed(Duration::days(10)).unwrap();

                    //TODO: change this to be configurable in toml / ask user to provide the team name
                    // it should match the same team that we check for above when determining if the user is a member
                    let team = github::get_team(&ctx.github, &"T-lang").await?.unwrap();

                    let mut current: BTreeMap<String, Option<UserStatus>> = BTreeMap::new();

                    for member in team.members {
                        current.insert(member.name, None);
                    }

                    current.insert(
                        user.login.clone(),
                        Some(UserStatus {
                            comment_id: "comment_id".to_string(),
                            text: "something".to_string(),
                            reversibility: Reversibility::Reversible,
                            resolution: Merge,
                        }),
                    );

                    let history: BTreeMap<String, Vec<UserStatus>> = BTreeMap::new();

                    insert_issue_decision_state(
                        &db,
                        &issue.number,
                        &user.login,
                        &start_date,
                        &end_date,
                        &current,
                        &history,
                        &reversibility,
                        &Merge,
                    )
                    .await?;

                    let metadata = serde_json::value::to_value(DecisionProcessActionMetadata {
                        message: "some message".to_string(),
                        get_issue_url: format!(
                            "{}/issues/{}",
                            issue.repository().url(),
                            issue.number
                        ),
                        status: Merge,
                    })
                    .unwrap();

                    insert_job(
                        &db,
                        &DECISION_PROCESS_JOB_NAME.to_string(),
                        &end_date,
                        &metadata,
                    )
                    .await?;

                    let comment = build_status_comment(&history, &current)?;

                    issue
                        .post_comment(&ctx.github, &comment)
                        .await
                        .context("merge vote comment")?;

                    Ok(())
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
    for (user, statuses) in history {
        let mut user_statuses = format!("\n| {} |", user);

        // previous stasuses
        for status in statuses {
            let status_item = format!(" ~~{}~~ ", status.resolution);
            user_statuses.push_str(&status_item);
        }

        // current status
        let user_resolution = match current.get(user) {
            Some(current_status) => {
                if let Some(status) = current_status {
                    format!("**{}**", status.resolution)
                } else {
                    "".to_string()
                }
            }
            None => bail!("user {} not present in current statuses list", user),
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
            comment_id = "the-comment-id".to_string(),
            text = "this is my argument for making this decision".to_string(),
            reversibility = Reversibility::Reversible,
            resolution = Resolution::Merge
        }

        mixin hold {
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
        | Barbara | ~~hold~~  ~~merge~~  **merge** |\n\
        | Niklaus | ~~merge~~  ~~hold~~  **merge** |"
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
        | Barbara | ~~hold~~  ~~merge~~  **merge** |\n\
        | Niklaus | ~~merge~~  ~~hold~~  **merge** |\n\
        | Tom |  |"
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
            "user Barbara not present in current statuses list"
        );
    }
}

// #[cfg(test)]
// mod tests {
//     use chrono::{Duration, Utc};
//     use pretty_assertions::assert_eq;

//     use super::*;

//     struct TestRenderer {}

//     impl LinkRenderer for TestRenderer {
//         fn render_link(&self, data: &UserStatus) -> String {
//             let issue_id = &data.issue_id;
//             let comment_id = &data.comment_id;

//             format!("http://example.com/issue/{issue_id}#comment={comment_id}")
//         }
//     }

//     /// Example 1
//     ///
//     /// https://lang-team.rust-lang.org/decision_process/examples.html#reversible-decision-merging-a-proposal
//     ///
//     /// * From the starting point of there not being any state, someone proposes
//     /// to merge a proposal
//     /// * then barbara holds
//     /// * 11 days pass
//     /// * barbara says merge, it immediatly merges
//     #[test]
//     fn example_merging_proposal() {
//         let team_members = vec![
//             "@Alan".to_owned(),
//             "@Barbara".to_owned(),
//             "@Grace".to_owned(),
//             "@Niklaus".to_owned(),
//         ];
//         let r = TestRenderer {};

//         // alan proposes to merge
//         let time1 = Utc::now();
//         let command = DecisionCommand::merge("@Alan".into(), "1".into(), "1".into());
//         let state = handle_command(None, command, Context::new(team_members.clone(), time1)).unwrap();

//         assert_eq!(state.period_start, time1);
//         assert_eq!(state.original_period_start, time1);
//         assert_eq!(
//             state.current_statuses,
//             vec![(
//                 "@Alan".into(),
//                 UserStatus::new("merge".into(), "1".into(), "1".into())
//             ),]
//             .into_iter()
//             .collect()
//         );
//         assert_eq!(state.status_history, HashMap::new());
//         assert_eq!(state.reversibility, Reversibility::Reversible);
//         assert_eq!(state.resolution, Custom("merge".into()));
//         assert_eq!(
//             state.render(&r),
//             include_str!("../../test/decision/res/01_merging_proposal__1.md")
//         );

//         // barbara holds
//         let time2 = Utc::now();
//         let command = DecisionCommand::hold("@Barbara".into(), "1".into(), "2".into());
//         let state = handle_command(
//             Some(state),
//             command,
//             Context::new(team_members.clone(), time2),
//         )
//         .unwrap();

//         assert_eq!(state.period_start, time1);
//         assert_eq!(state.original_period_start, time1);
//         assert_eq!(
//             state.current_statuses,
//             vec![
//                 (
//                     "@Alan".into(),
//                     UserStatus::new("merge".into(), "1".into(), "1".into())
//                 ),
//                 (
//                     "@Barbara".into(),
//                     UserStatus::new("hold".into(), "1".into(), "2".into())
//                 ),
//             ]
//             .into_iter()
//             .collect()
//         );
//         assert_eq!(state.status_history, HashMap::new());
//         assert_eq!(state.reversibility, Reversibility::Reversible);
//         assert_eq!(state.resolution, Custom("merge".into()));
//         assert_eq!(
//             state.render(&r),
//             include_str!("../../test/decision/res/01_merging_proposal__2.md")
//         );

//         // 11 days pass
//         let time3 = time2 + Duration::days(11);

//         // Barbara says merge, it immediatly merges
//         let command = DecisionCommand::merge("@Barbara".into(), "1".into(), "3".into());
//         let state = handle_command(Some(state), command, Context::new(team_members, time3)).unwrap();

//         assert_eq!(state.period_start, time1);
//         assert_eq!(state.original_period_start, time1);
//         assert_eq!(
//             state.current_statuses,
//             vec![
//                 (
//                     "@Alan".into(),
//                     UserStatus::new("merge".into(), "1".into(), "1".into())
//                 ),
//                 (
//                     "@Barbara".into(),
//                     UserStatus::new("merge".into(), "1".into(), "3".into())
//                 ),
//             ]
//             .into_iter()
//             .collect()
//         );
//         assert_eq!(
//             state.status_history,
//             vec![(
//                 "@Barbara".into(),
//                 vec![UserStatus::new("hold".into(), "1".into(), "2".into())]
//             ),]
//             .into_iter()
//             .collect()
//         );
//         assert_eq!(state.reversibility, Reversibility::Reversible);
//         assert_eq!(state.resolution, Custom("merge".into()));
//         assert_eq!(
//             state.render(&r),
//             include_str!("../../test/decision/01_merging_proposal__3.md")
//         );
//     }
// }
