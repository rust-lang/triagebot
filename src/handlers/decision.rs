use parser::command::decision::{
    Context, DecisionCommand, Error, Resolution, Reversibility, State, UserStatus, InvalidFirstCommand
};
use parser::command::decision::Resolution::*;
use std::collections::HashMap;

/// Applies a command to the current state and returns the next state
pub(super) async fn handle_command(
    ctx: &Context,
    _config: &DecisionConfig,
    event: &Event,
    cmd: DecisionCommand,
) -> Result<State, Error> {
    let DecisionCommand {
        user,
        issue_id,
        comment_id,
        disposition,
        reversibility,
    } = command;

    if let Some(state) = state {
        let name = match disposition {
            Hold => "hold".into(),
            Custom(name) => name,
        };

        let mut current_statuses = state.current_statuses;
        let mut status_history = state.status_history;

        if let Some(entry) = current_statuses.get_mut(&user) {
            let past = status_history.entry(user).or_insert(Vec::new());

            past.push(entry.clone());

            *entry = UserStatus::new(name, issue_id, comment_id);
        } else {
            current_statuses.insert(user, UserStatus::new("hold".into(), issue_id, comment_id));
        }

        Ok(State {
            current_statuses,
            status_history,
            ..state
        })
    } else {
        // no state, this is the first call to the decision process
        match disposition {
            Hold => Err(InvalidFirstCommand),

            Custom(name) => {
                let mut statuses = HashMap::new();

                statuses.insert(
                    user.clone(),
                    UserStatus::new(name.clone(), issue_id, comment_id),
                );

                let Context { team_members, now } = context;

                Ok(State {
                    initiator: user,
                    team_members,
                    period_start: now,
                    original_period_start: now,
                    current_statuses: statuses,
                    status_history: HashMap::new(),
                    reversibility,
                    resolution: Custom(name),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use pretty_assertions::assert_eq;

    use super::*;

    struct TestRenderer {}

    impl LinkRenderer for TestRenderer {
        fn render_link(&self, data: &UserStatus) -> String {
            let issue_id = &data.issue_id;
            let comment_id = &data.comment_id;

            format!("http://example.com/issue/{issue_id}#comment={comment_id}")
        }
    }

    /// Example 1
    ///
    /// https://lang-team.rust-lang.org/decision_process/examples.html#reversible-decision-merging-a-proposal
    ///
    /// * From the starting point of there not being any state, someone proposes
    /// to merge a proposal
    /// * then barbara holds
    /// * 11 days pass
    /// * barbara says merge, it immediatly merges
    #[test]
    fn example_merging_proposal() {
        let team_members = vec![
            "@Alan".to_owned(),
            "@Barbara".to_owned(),
            "@Grace".to_owned(),
            "@Niklaus".to_owned(),
        ];
        let r = TestRenderer {};

        // alan proposes to merge
        let time1 = Utc::now();
        let command = DecisionCommand::merge("@Alan".into(), "1".into(), "1".into());
        let state = handle_command(None, command, Context::new(team_members.clone(), time1)).unwrap();

        assert_eq!(state.period_start, time1);
        assert_eq!(state.original_period_start, time1);
        assert_eq!(
            state.current_statuses,
            vec![(
                "@Alan".into(),
                UserStatus::new("merge".into(), "1".into(), "1".into())
            ),]
            .into_iter()
            .collect()
        );
        assert_eq!(state.status_history, HashMap::new());
        assert_eq!(state.reversibility, Reversibility::Reversible);
        assert_eq!(state.resolution, Custom("merge".into()));
        assert_eq!(
            state.render(&r),
            include_str!("../../test/decision/res/01_merging_proposal__1.md")
        );

        // barbara holds
        let time2 = Utc::now();
        let command = DecisionCommand::hold("@Barbara".into(), "1".into(), "2".into());
        let state = handle_command(
            Some(state),
            command,
            Context::new(team_members.clone(), time2),
        )
        .unwrap();

        assert_eq!(state.period_start, time1);
        assert_eq!(state.original_period_start, time1);
        assert_eq!(
            state.current_statuses,
            vec![
                (
                    "@Alan".into(),
                    UserStatus::new("merge".into(), "1".into(), "1".into())
                ),
                (
                    "@Barbara".into(),
                    UserStatus::new("hold".into(), "1".into(), "2".into())
                ),
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(state.status_history, HashMap::new());
        assert_eq!(state.reversibility, Reversibility::Reversible);
        assert_eq!(state.resolution, Custom("merge".into()));
        assert_eq!(
            state.render(&r),
            include_str!("../../test/decision/res/01_merging_proposal__2.md")
        );

        // 11 days pass
        let time3 = time2 + Duration::days(11);

        // Barbara says merge, it immediatly merges
        let command = DecisionCommand::merge("@Barbara".into(), "1".into(), "3".into());
        let state = handle_command(Some(state), command, Context::new(team_members, time3)).unwrap();

        assert_eq!(state.period_start, time1);
        assert_eq!(state.original_period_start, time1);
        assert_eq!(
            state.current_statuses,
            vec![
                (
                    "@Alan".into(),
                    UserStatus::new("merge".into(), "1".into(), "1".into())
                ),
                (
                    "@Barbara".into(),
                    UserStatus::new("merge".into(), "1".into(), "3".into())
                ),
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(
            state.status_history,
            vec![(
                "@Barbara".into(),
                vec![UserStatus::new("hold".into(), "1".into(), "2".into())]
            ),]
            .into_iter()
            .collect()
        );
        assert_eq!(state.reversibility, Reversibility::Reversible);
        assert_eq!(state.resolution, Custom("merge".into()));
        assert_eq!(
            state.render(&r),
            include_str!("../../test/decision/01_merging_proposal__3.md")
        );
    }
}
