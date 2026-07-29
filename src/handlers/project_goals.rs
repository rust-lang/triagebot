use crate::{
    github::{
        self, Event, GithubClient, Issue, IssueCommentAction, IssueCommentEvent, IssuesAction,
        IssuesEvent,
    },
    handlers::Context,
    jobs::Job,
    team_data::TeamClient,
    zulip::{MessageApiRequest, api::Recipient, client::ZulipClient},
};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use itertools::Itertools;
use std::collections::BTreeMap;
use tracing as log;

const RUST_PROJECT_GOALS_REPO: &str = "rust-lang/rust-project-goals";
const C_TRACKING_ISSUE: &str = "C-tracking-issue";

const GOALS_STREAM: u64 = 435_869; // #project-goals
const TRIAGEBOT_TOPIC: &str = "Triagebot reports";
const MAX_ZULIP_TOPIC: usize = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct ZulipId(u64);

impl ZulipId {
    fn mention(self, muted: bool) -> String {
        if muted {
            format!("@_**|{}**", self.0)
        } else {
            format!("@**|{}**", self.0)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct GhUsername<'gh>(&'gh str);

impl<'gh> GhUsername<'gh> {
    fn link(self) -> String {
        format!("[@{login}](https://github.com/{login})", login = self.0,)
    }

    fn team_file_link(self) -> String {
        format!(
            "[@{login}](https://github.com/rust-lang/team/tree/main/people/{login}.toml)",
            login = self.0,
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct Owner<'gh> {
    github: GhUsername<'gh>,
    zulip: Option<ZulipId>,
}

impl<'gh> Owner<'gh> {
    async fn resolve(
        team: &TeamClient,
        github_id: u64,
        username: &'gh str,
    ) -> anyhow::Result<Self> {
        let zulip = team.github_to_zulip_id(github_id).await?.map(ZulipId);
        Ok(Self {
            github: GhUsername(username),
            zulip,
        })
    }

    fn display_mention(self, muted: bool) -> String {
        match self.zulip {
            Some(zulip_id) => zulip_id.mention(muted),
            None => self.github.link(),
        }
    }
}

#[derive(Clone, Debug)]
struct Owners<'gh>(Vec<Owner<'gh>>);

fn join_mentions(mentions: Vec<String>) -> String {
    match mentions.as_slice() {
        [] => "(none assigned)".to_owned(),
        [owner] => owner.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => {
            format!("{}, and {last}", rest.iter().join(", "))
        }
    }
}

impl<'gh> Owners<'gh> {
    async fn resolve(team: &TeamClient, issue: &'gh Issue) -> anyhow::Result<Self> {
        let mut owners = Vec::with_capacity(issue.assignees.len());
        for assignee in &issue.assignees {
            owners.push(Owner::resolve(team, assignee.id, &assignee.login).await?);
        }
        Ok(Self(owners))
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn has_multiple(&self) -> bool {
        self.0.len() > 1
    }

    fn reachable(&self) -> impl Iterator<Item = ZulipId> + '_ {
        self.0.iter().copied().filter_map(|owner| owner.zulip)
    }

    fn unreachable(&self) -> impl Iterator<Item = GhUsername<'gh>> + '_ {
        self.0
            .iter()
            .copied()
            .filter_map(|owner| owner.zulip.is_none().then_some(owner.github))
    }

    fn all_mentions(&self, muted: bool) -> String {
        join_mentions(
            self.0
                .iter()
                .copied()
                .map(|o| o.display_mention(muted))
                .collect_vec(),
        )
    }

    fn reachable_mentions(&self) -> Option<String> {
        let reachables = self.reachable().map(|id| id.mention(true)).collect_vec();
        if reachables.is_empty() {
            None
        } else {
            Some(join_mentions(reachables))
        }
    }

    fn unreachable_team_links(&self) -> String {
        join_mentions(
            self.unreachable()
                .map(GhUsername::team_file_link)
                .collect_vec(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum LastUpdate {
    Never,
    DaysAgo(i64),
}

impl LastUpdate {
    fn description(self) -> String {
        match self {
            Self::Never => "no updates so far".to_owned(),
            Self::DaysAgo(days) => format!("last update was {days} days ago"),
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct Reminder<'gh> {
    issue: u64,
    title: &'gh str,
    last_update: LastUpdate,
}

struct EvaluatedReminder<'gh> {
    reminder: Reminder<'gh>,
    requires_update: bool,
    invalid_schedule_reason: Option<String>,
}

impl<'gh> Reminder<'gh> {
    fn from_issue(issue: &'gh Issue, days_since_last_update: i64) -> Self {
        let last_update = if issue.comments.unwrap_or(0) <= 1 {
            LastUpdate::Never
        } else {
            LastUpdate::DaysAgo(days_since_last_update)
        };
        Self {
            issue: issue.number,
            title: &issue.title,
            last_update,
        }
    }

    fn list_item(&self) -> String {
        format!(
            "+ *{title}* (goals#{issue}) — {last_update}",
            title = self.title,
            issue = self.issue,
            last_update = self.last_update.description(),
        )
    }

    fn reference(&self) -> String {
        format!(
            "*{title}* (goals#{issue})",
            title = self.title,
            issue = self.issue,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum CustomSchedule {
    /// Runs on every job invocation (every week).
    Weekly = 0,
    /// Runs during even-numbered ISO weeks.
    Biweekly0 = 1,
    /// Runs during odd-numbered ISO weeks.
    Biweekly1 = 2,
    /// Runs on the first job invocation of each month.
    Monthly = 3,
}

#[derive(Clone, Debug)]
enum Schedule {
    Default,
    Custom(CustomSchedule),
    Invalid {
        fallback: CustomSchedule,
        reason: String,
    },
}

impl Schedule {
    fn from_issue(issue: &Issue) -> Self {
        const PING_FREQUENCY_LABELS: &[(&str, CustomSchedule)] = &[
            ("P-weekly", CustomSchedule::Weekly),
            ("P-biweekly-0", CustomSchedule::Biweekly0),
            ("P-biweekly-1", CustomSchedule::Biweekly1),
            ("P-monthly", CustomSchedule::Monthly),
        ];

        match PING_FREQUENCY_LABELS
            .iter()
            .filter_map(|&(label, value)| {
                issue
                    .labels
                    .iter()
                    .any(|l| l.name == label)
                    .then_some((label, value))
            })
            .collect::<Vec<_>>()
            .as_slice()
        {
            [] => Self::Default,
            [(_, frequency)] => Self::Custom(*frequency),
            multiple => {
                let (fallback_label, fallback) = multiple
                    .iter()
                    .copied()
                    .min_by_key(|(_, schedule)| *schedule)
                    .expect("multiple contains at least two schedules");

                Self::Invalid {
                    fallback,
                    reason: format!(
                        "multiple frequency labels are set: {}; falling back to `{fallback_label}`",
                        multiple.iter().map(|&(label, _)| label).join(", "),
                    ),
                }
            }
        }
    }
}

fn latest_biweekly_due_date(today: NaiveDate, parity: bool) -> NaiveDate {
    if today.iso_week().week() % 2 == parity as u32 {
        today
    } else {
        today - Duration::weeks(1)
    }
}

impl CustomSchedule {
    fn latest_due_date(self, today: NaiveDate) -> NaiveDate {
        match self {
            Self::Weekly => today,
            Self::Biweekly0 => latest_biweekly_due_date(today, false),
            Self::Biweekly1 => latest_biweekly_due_date(today, true),
            Self::Monthly => {
                let weeks_since_first_run = today.day0() / 7;
                today - Duration::weeks(i64::from(weeks_since_first_run))
            }
        }
    }
}

#[derive(Clone, Debug)]
struct MultipleOwners<'gh> {
    goal: Reminder<'gh>,
    owners: Owners<'gh>,
}

#[derive(Clone, Debug)]
struct InvalidSchedule<'gh> {
    goal: Reminder<'gh>,
    reason: String,
}

#[derive(Default)]
struct ReminderErrors<'gh> {
    unowned: Vec<Reminder<'gh>>,
    multiply_owned: Vec<MultipleOwners<'gh>>,
    missing_zulip: Vec<MultipleOwners<'gh>>,
    invalid_schedules: Vec<InvalidSchedule<'gh>>,
}

impl ReminderErrors<'_> {
    fn is_empty(&self) -> bool {
        self.unowned.is_empty()
            && self.multiply_owned.is_empty()
            && self.missing_zulip.is_empty()
            && self.invalid_schedules.is_empty()
    }
}

#[derive(Default)]
struct ReminderPlan<'gh> {
    goals_by_owner: BTreeMap<ZulipId, Vec<Reminder<'gh>>>,
    errors: ReminderErrors<'gh>,
}

impl<'gh> ReminderPlan<'gh> {
    fn add_invalid_schedule(&mut self, goal: Reminder<'gh>, reason: String) {
        self.errors
            .invalid_schedules
            .push(InvalidSchedule { goal, reason });
    }

    fn add_goal(&mut self, goal: Reminder<'gh>, owners: Owners<'gh>) {
        if owners.is_empty() {
            self.errors.unowned.push(goal);
            return;
        }

        let goal_with_owners = MultipleOwners {
            goal: goal.clone(),
            owners: owners.clone(),
        };

        if owners.has_multiple() {
            self.errors.multiply_owned.push(goal_with_owners.clone());
        }

        if owners.unreachable().next().is_some() {
            self.errors.missing_zulip.push(goal_with_owners);
        }

        for owner in owners.reachable() {
            self.goals_by_owner
                .entry(owner)
                .or_default()
                .push(goal.clone());
        }
    }
}

fn owner_message(owner: ZulipId, goals: &[Reminder<'_>]) -> String {
    format!(
        r#"
Hi {owner}!

This is a reminder to post an update on your goals:

{goals}

Some questions to guide you (you don't have to follow this format):

+ What has happened since your last update?
+ Are there any relevant PRs, issues, docs, or discussions to link?
+ Are you blocked on any issue, PR, teams?
+ Do you need help or feedback? Where should people look?
+ What do you plan to work on before the next update?

Even if there's little to say, a brief message provides reassurance that the goal is still alive.

Please leave your updates as comments on the tracking issues. Thanks! <3
"#,
        owner = owner.mention(false),
        goals = goals.iter().map(Reminder::list_item).join("\n"),
    )
}

fn unowned_errors(goals: &[Reminder<'_>]) -> String {
    let goals = goals
        .iter()
        .map(|goal| format!("+ {goal}", goal = goal.reference()))
        .join("\n");

    format!(
        r#"
The following goals have no owner assigned:

{goals}

Please assign an owner and reach out to them!
"#
    )
}

fn multiple_owner_errors(goals: &[MultipleOwners<'_>]) -> String {
    let goals = goals
        .iter()
        .map(|entry| {
            format!(
                "+ {goal} — assigned to {owners}",
                goal = entry.goal.reference(),
                owners = entry.owners.all_mentions(true),
            )
        })
        .join("\n");

    format!(
        r#"
The following goals have more than one owner assigned:

{goals}

A goal should have exactly one owner. All owners with a Zulip account were still notified separately.
"#
    )
}

fn missing_zulip_errors(goals: &[MultipleOwners<'_>]) -> String {
    let goals = goals
        .iter()
        .map(|entry| {
            format!(
                "+ {goal} — missing Zulip account: {unreachable}\n  {notified}",
                goal = entry.goal.reference(),
                unreachable = entry.owners.unreachable_team_links(),
                notified = match entry.owners.reachable_mentions() {
                    None => "No existing owner was notified on Zulip.".to_owned(),
                    Some(owners) => format!("{owners} got notified on Zulip."),
                }
            )
        })
        .join("\n");

    format!(
        r#"
The following goal owners were not pinged because they don't have a Zulip account specified in the `team` repo:

{goals}

Please make sure to register their `zulip-id` and reach out to them!
"#
    )
}

fn invalid_schedule_errors(errors: &[InvalidSchedule<'_>]) -> String {
    let errors = errors
        .iter()
        .map(|error| format!("+ {} — {}", error.goal.reference(), error.reason,))
        .join("\n");

    format!(
        r#"
The following goals have invalid ping-schedule labels:

{errors}

Use exactly one frequency label (`P-weekly`, `P-biweekly-0`, `P-biweekly-1`, or `P-monthly`).
"#
    )
}

fn error_message(errors: &ReminderErrors<'_>) -> String {
    let mut sections = Vec::new();

    if !errors.unowned.is_empty() {
        sections.push(unowned_errors(&errors.unowned));
    }
    if !errors.multiply_owned.is_empty() {
        sections.push(multiple_owner_errors(&errors.multiply_owned));
    }
    if !errors.missing_zulip.is_empty() {
        sections.push(missing_zulip_errors(&errors.missing_zulip));
    }
    if !errors.invalid_schedules.is_empty() {
        sections.push(invalid_schedule_errors(&errors.invalid_schedules));
    }

    format!(
        r#"
Hi @*T-goals*!

{}
"#,
        sections.iter().join("\n\n---\n\n"),
    )
}

fn default_update_required(issue: &Issue, now: DateTime<Utc>, days_threshold: i64) -> bool {
    let comments = issue.comments.unwrap_or(0);
    let days_since_last_update = (now - issue.updated_at).num_days();

    days_since_last_update >= days_threshold || comments <= 1
}

fn scheduled_update_required(
    issue: &Issue,
    now: DateTime<Utc>,
    schedule: CustomSchedule,
) -> bool {
    let due_date = schedule.latest_due_date(now.date_naive());
    let has_real_update = issue.comments.unwrap_or(0) > 1;
    let updated_after_due_date = issue.updated_at.date_naive() >= due_date;

    !has_real_update || !updated_after_due_date
}

fn evaluate<'gh>(
    issue: &'gh Issue,
    now: DateTime<Utc>,
    days_threshold: i64,
) -> EvaluatedReminder<'gh> {
    let days_since_last_update = (now - issue.updated_at).num_days();

    log::debug!(
        "issue #{}: days_since_last_comment = {} days, comments = {}",
        issue.number,
        days_since_last_update,
        issue.comments.unwrap_or(0),
    );

    let (requires_update, invalid_schedule_reason) = match Schedule::from_issue(issue) {
        Schedule::Default => (
            default_update_required(issue, now, days_threshold),
            None,
        ),
        Schedule::Custom(schedule) => (scheduled_update_required(issue, now, schedule), None),
        Schedule::Invalid { fallback, reason } => (
            scheduled_update_required(issue, now, fallback),
            Some(reason),
        ),
    };

    EvaluatedReminder {
        reminder: Reminder::from_issue(issue, days_since_last_update),
        requires_update,
        invalid_schedule_reason,
    }
}

async fn build_plan<'gh>(
    issues: &'gh [Issue],
    team: &TeamClient,
    days_threshold: i64,
) -> anyhow::Result<ReminderPlan<'gh>> {
    let now = Utc::now();
    let mut plan = ReminderPlan::default();

    for issue in issues {
        let evaluation = evaluate(issue, now, days_threshold);

        if let Some(reason) = evaluation.invalid_schedule_reason {
            plan.add_invalid_schedule(evaluation.reminder, reason);
        }

        if !evaluation.requires_update {
            continue;
        }

        let owners = Owners::resolve(team, issue).await?;
        plan.add_goal(evaluation.reminder, owners);
    }

    Ok(plan)
}

async fn send_dm(
    zulip: &ZulipClient,
    owner: ZulipId,
    content: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    if dry_run {
        log::debug!("(DRY) Would send DM to user {}: {}", owner.0, content,);
        return Ok(());
    }

    MessageApiRequest {
        recipient: Recipient::Private {
            id: owner.0,
            email: "",
        },
        content,
    }
    .send(zulip)
    .await?;

    Ok(())
}

async fn send_triagebot_topic(
    zulip: &ZulipClient,
    content: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    if dry_run {
        log::debug!(
            "(DRY) Would send to topic {GOALS_STREAM}>{TRIAGEBOT_TOPIC}: {}",
            content,
        );
        return Ok(());
    }

    MessageApiRequest {
        recipient: Recipient::Stream {
            id: GOALS_STREAM,
            topic: TRIAGEBOT_TOPIC,
        },
        content,
    }
    .send(zulip)
    .await?;

    Ok(())
}

async fn execute_plan(
    zulip: &ZulipClient,
    plan: ReminderPlan<'_>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let mut total_owners = 0;
    let total_goals = plan
        .goals_by_owner
        .values()
        .flatten()
        .map(|goal| goal.issue)
        .unique()
        .count();
    let mut total_errors = 0;

    for (owner, goals) in plan.goals_by_owner {
        send_dm(zulip, owner, &owner_message(owner, &goals), dry_run).await?;
        total_owners += 1;
    }

    if !plan.errors.is_empty() {
        send_triagebot_topic(zulip, &error_message(&plan.errors), dry_run).await?;

        total_errors += plan.errors.unowned.len()
            + plan.errors.multiply_owned.len()
            + plan.errors.missing_zulip.len()
            + plan.errors.invalid_schedules.len();
    }

    send_triagebot_topic(
        zulip,
        &format!(
            r#"
Weekly run finished.

{total_owners} owners have been notified about {total_goals} goals.

{total_errors} errors happened in the process.

Until next week! <3
        "#
        ),
        dry_run,
    )
    .await?;

    Ok(())
}

fn is_tracking_issue(issue: &Issue) -> bool {
    issue
        .labels
        .iter()
        .any(|label| label.name == C_TRACKING_ISSUE)
}

async fn tracking_issues(gh: &GithubClient) -> anyhow::Result<Vec<Issue>> {
    gh.repository(RUST_PROJECT_GOALS_REPO)
        .await?
        .get_issues(
            gh,
            &github::issue_query::Query {
                filters: vec![("state", "open"), ("is", "issue")],
                include_labels: vec![C_TRACKING_ISSUE],
                exclude_labels: vec![],
            },
        )
        .await
}

pub async fn ping_project_goals_owners(
    gh: &GithubClient,
    zulip: &ZulipClient,
    team: &TeamClient,
    dry_run: bool,
    days_threshold: i64,
) -> anyhow::Result<()> {
    let issues = tracking_issues(gh).await?;
    let plan = build_plan(&issues, team, days_threshold).await?;
    execute_plan(zulip, plan, dry_run).await
}

pub struct ProjectGoalsUpdateJob;

#[async_trait]
impl Job for ProjectGoalsUpdateJob {
    fn name(&self) -> &'static str {
        "project_goals_update_job"
    }

    async fn run(&self, ctx: &Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        let now = Utc::now();
        let days_threshold = i64::from(now.day() + 7);

        ping_project_goals_owners(&ctx.github, &ctx.zulip, &ctx.team, false, days_threshold).await
    }
}

/// Returns true if the GitHub user is part of the Goals team.
pub async fn is_goals_member(team_client: &TeamClient, github_id: u64) -> anyhow::Result<bool> {
    const GOALS_TEAM: &str = "goals";

    let team = match team_client.get_team(GOALS_TEAM).await? {
        Some(team) => team,
        None => {
            log::info!("team ({GOALS_TEAM}) failed to resolve to a known team");
            return Ok(false);
        }
    };

    Ok(team
        .members
        .into_iter()
        .any(|member| member.github_id == github_id))
}

fn goal_zulip_topic(issue: &Issue) -> String {
    let goal_number = format!("(goals#{})", issue.number);
    let mut title = String::new();
    for word in issue.title.split_whitespace() {
        if title.len() + word.len() + 1 + goal_number.len() >= MAX_ZULIP_TOPIC {
            break;
        }
        title.push_str(word);
        title.push(' ');
    }
    title.push_str(&goal_number);
    assert!(title.len() < MAX_ZULIP_TOPIC);
    title
}

async fn create_goal_topic(issue: &Issue, ctx: &Context) -> anyhow::Result<()> {
    if !is_tracking_issue(issue) {
        return Ok(());
    }

    let owners = Owners::resolve(&ctx.team, issue).await?;
    let topic = goal_zulip_topic(issue);
    let content = format!(
        "Goal *{title}* (goals#{number}) has been accepted. It's owned by {owners}.",
        title = issue.title,
        number = issue.number,
        owners = owners.all_mentions(false),
    );

    MessageApiRequest {
        recipient: Recipient::Stream {
            id: GOALS_STREAM,
            topic: &topic,
        },
        content: &content,
    }
    .send(&ctx.zulip)
    .await?;

    Ok(())
}

fn quote_fence(text: &str) -> String {
    let mut ticks = "````".to_owned();

    while text.contains(&ticks) {
        ticks.push('`');
    }

    ticks
}

async fn echo_comment_to_zulip(
    issue: &Issue,
    comment: &github::Comment,
    ctx: &Context,
) -> anyhow::Result<()> {
    if !is_tracking_issue(issue) {
        return Ok(());
    }

    let author = Owner::resolve(&ctx.team, comment.user.id, &comment.user.login).await?;
    let text = &comment.body;

    let content = format!(
        "[Comment posted]({url}) on goals#{number} by {author}:\n\
         {ticks}quote\n\
         {text}\n\
         {ticks}",
        url = comment.html_url,
        number = issue.number,
        author = author.display_mention(false),
        ticks = quote_fence(&text),
    );

    MessageApiRequest {
        recipient: Recipient::Stream {
            id: GOALS_STREAM,
            topic: &goal_zulip_topic(issue),
        },
        content: &content,
    }
    .send(&ctx.zulip)
    .await?;

    Ok(())
}

pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    if event.repo().full_name != RUST_PROJECT_GOALS_REPO {
        return Ok(());
    }

    match event {
        Event::Issue(IssuesEvent {
            action: IssuesAction::Opened,
            issue,
            ..
        }) => create_goal_topic(issue, ctx).await,

        Event::IssueComment(IssueCommentEvent {
            action: IssueCommentAction::Created,
            issue,
            comment,
            ..
        }) => echo_comment_to_zulip(issue, comment, ctx).await,

        _ => Ok(()),
    }
}
