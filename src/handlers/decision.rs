use anyhow::Context as Ctx;
use parser::command::decision::{DecisionCommand, ParseError};
use crate::{
    config::DecisionConfig,
    github::{self, Event},
    handlers::Context,
    interactions::ErrorComment,
    db::decision_state::*
};
use std::collections::HashMap;
use chrono::{DateTime, Duration, Utc};

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

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &DecisionConfig,
    event: &Event,
    cmd: DecisionCommand,
) -> anyhow::Result<()> {
    let DecisionCommand {
        resolution,
        reversibility
    } = cmd;

    let issue = event.issue().unwrap();

    let is_team_member = event
        .user()
        .is_team_member(&ctx.github)
        .await
        .unwrap_or(false);

    if !is_team_member {
        let cmnt = ErrorComment::new(&issue, "Only team members can be part of the decision process.");
        cmnt.post(&ctx.github).await?;
        return Ok(());
    }

    match get_decision_state(issue.id) {
        Some(state) => {
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
            Ok();
        },
        None => {
            match resolution {
                Hold => Err(ParseError::InvalidFirstCommand),
                Merge => {
                    let start_date = chrono::Utc::now().into();
                    let end_date = start_date.checked_add_signed(Duration::days(10)).unwrap();

                    let current_statuses = HashMap::new();
                    let status_history = HashMap::new();

                    let team = github::get_team(&ctx.github, &"T-lang"); // change this to be configurable in toml?

                    insert_decision_state(
                        db,
                        issue.id,
                        user.login,
                        start_date,
                        end_date,
                        current_statuses,
                        status_history,
                        reversibility,
                        resolution,
                    );

                    let comment = format!(
                        "Wow, it looks like you want to merge this, {}.", event.user().login
                    );
                
                    let comment = format!(
                        "| Team member | State |\n|-------------|-------|\n| julmontesdeoca | merge |\n| mcass19 |  |");
                
                    issue
                        .post_comment(&ctx.github, &comment)
                        .await
                        .context("merge vote comment")?;

                    Ok();
                }
            }
        }
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
