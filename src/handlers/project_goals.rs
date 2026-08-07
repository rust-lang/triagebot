use crate::{
    github::{
        self, Event, GithubClient, Issue, IssueCommentAction, IssueCommentEvent, IssuesAction,
        IssuesEvent, queries::open_goal_issues::GoalIssue,
    },
    handlers::Context,
    jobs::Job,
    team_data::TeamClient,
    zulip::{MessageApiRequest, api::Recipient, client::ZulipClient},
};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use itertools::Itertools;
use std::collections::BTreeMap;
use tracing as log;

const RUST_PROJECT_GOALS_REPO: &str = "rust-lang/rust-project-goals";
const GOALS_TEAM: &str = "goals";

const FIRST_REPORT_GRACE_DAYS: i64 = 7;
const REPORT_LABELS: &[(&str, Period)] = &[
    ("R-every-week", Period::EveryWeek),
    ("R-every-2-weeks", Period::Every2Weeks),
    ("R-every-4-weeks", Period::Every4Weeks),
];

const GOALS_STREAM: u64 = 435_869; // #project-goals
const GOALS_META_STREAM: u64 = 478_266; // #project-goals/meta
const TRIAGEBOT_TOPIC: &str = "triagebot reports";
const MAX_ZULIP_TOPIC: usize = 60;

/// The weekday of the job execution (must match [`crate::jobs`]).
const JOB_WEEKDAY: Weekday = Weekday::Thu;
/// The UTC hour of the job execution (must match [`crate::jobs`]).
const JOB_UTC_HOUR: u32 = 14;
/// The UTC minute of the job execution (must match [`crate::jobs`]).
const JOB_UTC_MINUTE: u32 = 0;

/// An arbitrary date to keep reporting periods anchored.
///
/// The phase of the biweekly and 4-week periods depends on this day.
const EPOCH: NaiveDate = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();

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
        format!("[@{login}](https://github.com/{login})", login = self.0)
    }

    fn team_link(self) -> String {
        format!(
            "[@{login}](https://github.com/rust-lang/team/tree/main/people/{login}.toml)",
            login = self.0,
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum OwnerContact {
    Reachable(ZulipId),
    MissingZulipId,
    MissingTeamEntry,
}

#[derive(Clone, Copy, Debug)]
struct Owner<'gh> {
    github: GhUsername<'gh>,
    contact: OwnerContact,
}

impl<'gh> Owner<'gh> {
    async fn from_username(team: &TeamClient, username: &'gh str) -> anyhow::Result<Self> {
        Ok(Self {
            github: GhUsername(username),
            contact: match team.get_gh_id_from_username(username).await? {
                Some(gh_id) => match team.github_to_zulip_id(gh_id).await? {
                    Some(zulip_id) => OwnerContact::Reachable(ZulipId(zulip_id)),
                    None => OwnerContact::MissingZulipId,
                },
                None => OwnerContact::MissingTeamEntry,
            },
        })
    }

    async fn from_id_and_username(
        team: &TeamClient,
        gh_id: u64,
        username: &'gh str,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            github: GhUsername(username),
            contact: match team.github_to_zulip_id(gh_id).await? {
                Some(zulip_id) => OwnerContact::Reachable(ZulipId(zulip_id)),
                None => OwnerContact::MissingZulipId,
            },
        })
    }

    fn display_mention(self, muted: bool) -> String {
        match self.contact {
            OwnerContact::Reachable(zulip_id) => zulip_id.mention(muted),
            OwnerContact::MissingZulipId | OwnerContact::MissingTeamEntry => self.github.link(),
        }
    }
}

fn join_mentions(mentions: Vec<String>) -> Option<String> {
    let joined = match mentions.as_slice() {
        [] => return None,
        [owner] => owner.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.iter().join(", ")),
    };
    Some(joined)
}

#[derive(Debug)]
struct Owners<'gh>(Vec<Owner<'gh>>);

impl<'gh> Owners<'gh> {
    async fn resolve_goal(team: &TeamClient, issue: &'gh GoalIssue) -> anyhow::Result<Self> {
        let mut owners = Vec::with_capacity(issue.assignees.len());
        for username in &issue.assignees {
            owners.push(Owner::from_username(team, username).await?);
        }
        Ok(Self(owners))
    }

    async fn resolve_event(team: &TeamClient, issue: &'gh Issue) -> anyhow::Result<Self> {
        let mut owners = Vec::with_capacity(issue.assignees.len());
        for assignee in &issue.assignees {
            owners.push(Owner::from_id_and_username(team, assignee.id, &assignee.login).await?);
        }
        Ok(Self(owners))
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn has_multiple(&self) -> bool {
        self.0.len() > 1
    }

    fn has_missing_zulip_id(&self) -> bool {
        self.0
            .iter()
            .any(|owner| matches!(owner.contact, OwnerContact::MissingZulipId))
    }

    fn has_missing_team_entry(&self) -> bool {
        self.0
            .iter()
            .any(|owner| matches!(owner.contact, OwnerContact::MissingTeamEntry))
    }

    fn has_problem(&self) -> bool {
        self.has_multiple() || self.has_missing_zulip_id() || self.has_missing_team_entry()
    }

    fn reachable(&self) -> impl Iterator<Item = ZulipId> + '_ {
        self.0.iter().filter_map(|owner| match owner.contact {
            OwnerContact::Reachable(zulip_id) => Some(zulip_id),
            OwnerContact::MissingZulipId | OwnerContact::MissingTeamEntry => None,
        })
    }

    fn missing_zulip_ids(&self) -> impl Iterator<Item = GhUsername<'gh>> + '_ {
        self.0.iter().filter_map(|owner| {
            matches!(owner.contact, OwnerContact::MissingZulipId).then_some(owner.github)
        })
    }

    fn missing_team_entries(&self) -> impl Iterator<Item = GhUsername<'gh>> + '_ {
        self.0.iter().filter_map(|owner| {
            matches!(owner.contact, OwnerContact::MissingTeamEntry).then_some(owner.github)
        })
    }

    fn all_mentions(&self, muted: bool) -> Option<String> {
        join_mentions(
            self.0
                .iter()
                .copied()
                .map(|owner| owner.display_mention(muted))
                .collect_vec(),
        )
    }

    fn reachable_mentions(&self) -> Option<String> {
        join_mentions(self.reachable().map(|id| id.mention(true)).collect_vec())
    }

    fn missing_zulip_team_links(&self) -> Option<String> {
        join_mentions(
            self.missing_zulip_ids()
                .map(GhUsername::team_link)
                .collect_vec(),
        )
    }

    fn missing_team_entry_links(&self) -> Option<String> {
        join_mentions(
            self.missing_team_entries()
                .map(GhUsername::link)
                .collect_vec(),
        )
    }
}

/// Every goal has its own reporting schedule.
/// This is how often the goal owner is prompted to author an update.
///
/// This is set via a label (see [`REPORT_LABELS`]) on the goal's tracking issue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Period {
    /// Start a new reporting period every week.
    EveryWeek = 0,
    /// Start a new reporting period every 2 weeks.
    Every2Weeks = 1,
    /// Start a new reporting period every 4 weeks.
    Every4Weeks = 2,
}

impl Period {
    fn weeks(self) -> i64 {
        match self {
            Self::EveryWeek => 1,
            Self::Every2Weeks => 2,
            Self::Every4Weeks => 4,
        }
    }

    fn adjective(self) -> &'static str {
        match self {
            Self::EveryWeek => "weekly",
            Self::Every2Weeks => "biweekly",
            Self::Every4Weeks => "4-week",
        }
    }

    /// Returns the starting date of the period that includes this day.
    ///
    /// Every reporting period begins on a [`JOB_WEEKDAY`]
    /// and lasts [`Period::weeks`], depending on the goal.
    ///
    /// Biweekly and 4-week cycles are aligned to [`EPOCH`].
    fn start(self, today: NaiveDate) -> NaiveDate {
        // Depending on the chosen date, `EPOCH` may not fall on the `JOB_WEEKDAY`.
        // `days_until_job` is needed to calculate an anchor from the `EPOCH` that
        // falls on the `JOB_WEEKDAY` and can be used to compute the relevant dates.
        let epoch_weekday = i64::from(EPOCH.weekday().num_days_from_monday());
        let job_weekday = i64::from(JOB_WEEKDAY.num_days_from_monday());
        let days_until_job = (job_weekday - epoch_weekday).rem_euclid(7);

        // Dates are computed relative to this date.
        let anchor = EPOCH + Duration::days(days_until_job);

        // The number of full weeks since the anchor.
        let weeks_since_anchor = today.signed_duration_since(anchor).num_weeks();
        let period_weeks = self.weeks();
        // The number of full periods that passed since the anchor date.
        let periods_since_anchor = weeks_since_anchor.div_euclid(period_weeks);
        // The number of weeks since the anchor, quantized to the period.
        let weeks_since_anchor = periods_since_anchor * period_weeks;

        anchor + Duration::weeks(weeks_since_anchor)
    }

    /// Returns the starting date of the next period,
    /// i.e. this period's starting date plus the duration of a period.
    fn next_start(self, period_start: NaiveDate) -> NaiveDate {
        period_start + Duration::weeks(self.weeks())
    }
}

#[derive(Debug)]
struct Schedule {
    period: Period,
    conflict: Option<String>,
}

impl Schedule {
    fn from_issue(issue: &GoalIssue) -> Self {
        let selected = REPORT_LABELS
            .iter()
            .filter_map(|&(label, period)| {
                issue
                    .labels
                    .iter()
                    .any(|issue_label| issue_label == label)
                    .then_some((label, period))
            })
            .collect_vec();

        match selected.as_slice() {
            [] => Self {
                period: Period::Every4Weeks,
                conflict: None,
            },
            [(_, period)] => Self {
                period: *period,
                conflict: None,
            },
            multiple => {
                let (minimal_label, minimal) = multiple
                    .iter()
                    .copied()
                    .min_by_key(|(_, period)| *period)
                    .expect("multiple contains at least two schedules");
                Self {
                    period: minimal,
                    conflict: Some(format!(
                        "{} (`{minimal_label}` was used)",
                        multiple
                            .iter()
                            .map(|(label, _)| format!("`{label}`"))
                            .join(", "),
                    )),
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Goal<'gh> {
    issue: u64,
    title: &'gh str,
    created_at: DateTime<Utc>,
    last_comment_at: Option<DateTime<Utc>>,
}

impl<'gh> Goal<'gh> {
    fn from_issue(issue: &'gh GoalIssue) -> Self {
        Self {
            issue: issue.number,
            title: &issue.title,
            created_at: issue.created_at,
            last_comment_at: issue.last_comment.as_ref().map(|c| c.created_at),
        }
    }

    /// Returns a string representing an issue in the `rust-lang/rust-project-goals` repo.
    /// Zulip recognizes strings like `goals#123` and turns them into links.
    fn link(&self) -> String {
        format!("goals#{number}", number = self.issue)
    }

    /// Same as [`Goal::link`], but also includes the goal title.
    fn named_link(&self) -> String {
        format!(
            "**{title}** (goals#{number})",
            title = self.title,
            number = self.issue
        )
    }

    fn latest_update(&self) -> String {
        match self.last_comment_at {
            None => format!(
                "goal started: {} (no updates so far)",
                display_datetime(self.created_at)
            ),
            Some(dt) => {
                format!("latest update: {}", display_datetime(dt))
            }
        }
    }
}

fn display_job_date(date: NaiveDate) -> String {
    format!(
        "<time:{}T{JOB_UTC_HOUR:02}:{JOB_UTC_MINUTE:02}+00:00>",
        date.format("%Y-%m-%d"),
    )
}

fn display_datetime(date: DateTime<Utc>) -> String {
    format!("<time:{}>", date.format("%Y-%m-%dT%H:%M%:z"),)
}

#[derive(Clone, Copy, Debug)]
struct Reminder<'gh> {
    goal: Goal<'gh>,
    /// The reporting period of this goal.
    period: Period,
    /// The start date of this goal's current reporting period.
    period_start: NaiveDate,
    /// The start date of this goal's next reporting period.
    ///
    /// (This acts as a deadline for the current update.)
    next_period_start: NaiveDate,
}

impl<'gh> Reminder<'gh> {
    fn from_issue(issue: &'gh GoalIssue, now: DateTime<Utc>) -> (Self, Option<String>) {
        let schedule = Schedule::from_issue(issue);
        let today = now.date_naive();
        let period_start = schedule.period.start(today);
        (
            Self {
                goal: Goal::from_issue(issue),
                period: schedule.period,
                period_start,
                next_period_start: schedule.period.next_start(period_start),
            },
            schedule.conflict,
        )
    }

    fn is_required(&self, now: DateTime<Utc>) -> bool {
        // Give new goals a grace period before reminders begin.
        let grace_end = self.goal.created_at + Duration::days(FIRST_REPORT_GRACE_DAYS);

        if now < grace_end {
            return false;
        }

        self.goal
            .last_comment_at
            .is_none_or(|d| d.date_naive() < self.period_start)
    }

    fn list_item(&self) -> String {
        format!(
            "+ {goal}\n  - {latest}\n  - next *{period}* cycle starts {next}",
            goal = self.goal.named_link(),
            latest = self.goal.latest_update(),
            next = display_job_date(self.next_period_start),
            period = self.period.adjective(),
        )
    }
}

#[derive(Debug)]
struct OwnershipProblem<'gh> {
    goal: Goal<'gh>,
    owners: Owners<'gh>,
}

#[derive(Debug)]
struct PeriodConflict<'gh> {
    goal: Goal<'gh>,
    reason: String,
}

#[derive(Default)]
struct ReminderErrors<'gh> {
    unowned: Vec<Goal<'gh>>,
    ownership: Vec<OwnershipProblem<'gh>>,
    schedule: Vec<PeriodConflict<'gh>>,
}

impl ReminderErrors<'_> {
    fn is_empty(&self) -> bool {
        self.unowned.is_empty() && self.ownership.is_empty() && self.schedule.is_empty()
    }

    fn count(&self) -> usize {
        self.unowned.len()
            + self.schedule.len()
            + self
                .ownership
                .iter()
                .map(|problem| {
                    problem.owners.has_multiple() as usize
                        + problem.owners.has_missing_zulip_id() as usize
                        + problem.owners.has_missing_team_entry() as usize
                })
                .sum::<usize>()
    }
}

#[derive(Default)]
struct ReminderPlan<'gh> {
    goals_by_owner: BTreeMap<ZulipId, Vec<Reminder<'gh>>>,
    errors: ReminderErrors<'gh>,
}

impl<'gh> ReminderPlan<'gh> {
    fn add_conflicts(&mut self, goal: Goal<'gh>, reason: String) {
        self.errors.schedule.push(PeriodConflict { goal, reason });
    }

    fn add_goal(&mut self, reminder: Reminder<'gh>, owners: Owners<'gh>) {
        if owners.is_empty() {
            self.errors.unowned.push(reminder.goal);
            return;
        }

        for owner in owners.reachable() {
            self.goals_by_owner.entry(owner).or_default().push(reminder);
        }

        if owners.has_problem() {
            self.errors.ownership.push(OwnershipProblem {
                goal: reminder.goal,
                owners,
            });
        }
    }
}

fn owner_message(owner: ZulipId, goals: &[Reminder<'_>]) -> String {
    format!(
        r#"
Hi {owner}!

This is your reminder to post updates for the following goals:

{goals}

Some questions to guide you (you don't have to follow this):

+ What has happened since your last update?
+ Are there any relevant PRs, issues, docs, or discussions to link?
+ Are you blocked on any issue, PR, or team?
+ Do you need help or feedback? Where should people look?
+ What do you plan to work on before the next update?

Even if there's little to say, a brief message provides reassurance that the goal is still alive.

Please leave your updates as comments on the tracking issues. Thanks! <3

---

*Note: Two- and four-week goals are pinged weekly until an update is posted for the current reporting period.*

*By default, the reporting period is 4 weeks. If you'd like to post updates more often, you can override the period per goal by labeling the issue with `R-every-week`, `R-every-2-weeks`, or `R-every-4-weeks`.*
"#,
        owner = owner.mention(false),
        goals = goals.iter().map(Reminder::list_item).join("\n"),
    )
}

fn unowned_errors(goals: &[Goal<'_>]) -> String {
    format!(
        r#"
The following goals have no owner assigned:

{unowned}

Please assign an owner and reach out to them!
"#,
        unowned = goals
            .iter()
            .map(|g| format!("+ {}", g.named_link()))
            .join("\n")
    )
}

fn multiple_owner_warnings(problems: &[OwnershipProblem<'_>]) -> String {
    format!(
        r#"
The following goals have more than one owner assigned:

{multiple_owner}

A goal should have exactly one owner. All owners with a Zulip account were still notified separately.
"#,
        multiple_owner = problems
            .iter()
            .filter(|p| p.owners.has_multiple())
            .map(|p| {
                format!(
                    "+ {goal}: {owners}",
                    goal = p.goal.link(),
                    owners = p.owners.all_mentions(true).expect("has multiple"),
                )
            })
            .join("\n")
    )
}

fn missing_zulip_errors(problems: &[OwnershipProblem<'_>]) -> String {
    format!(
        r#"
The following goal owners were not pinged because they don't have a Zulip account specified in the `team` repo:

{missing_zulip}

Please make sure to register their `zulip-id` and reach out to them!
"#,
        missing_zulip = problems
            .iter()
            .filter(|p| p.owners.has_missing_zulip_id())
            .map(|p| {
                format!(
                    "+ {goal}: {unreachable}\n  - {notified}",
                    goal = p.goal.link(),
                    unreachable = p
                        .owners
                        .missing_zulip_team_links()
                        .expect("has missing Zulip ID"),
                    notified = match p.owners.reachable_mentions() {
                        None => "Nobody was notified on Zulip.".to_owned(),
                        Some(owners) => format!("{owners} got notified on Zulip."),
                    }
                )
            })
            .join("\n")
    )
}

fn missing_team_entry_warnings(problems: &[OwnershipProblem<'_>]) -> String {
    format!(
        r#"
The following assignees could not be found in the `team` repo, so Triagebot could not look up their Zulip accounts:

{missing_team_entries}

Please check the assignee usernames and their entries in the `team` repo.
"#,
        missing_team_entries = problems
            .iter()
            .filter(|p| p.owners.has_missing_team_entry())
            .map(|p| {
                format!(
                    "+ {goal}: {owners}\n  - {notified}",
                    goal = p.goal.link(),
                    owners = p
                        .owners
                        .missing_team_entry_links()
                        .expect("has missing team entry"),
                    notified = match p.owners.reachable_mentions() {
                        None => "Nobody was notified on Zulip.".to_owned(),
                        Some(owners) => format!("{owners} got notified on Zulip."),
                    },
                )
            })
            .join("\n")
    )
}

fn schedule_warnings(conflicts: &[PeriodConflict<'_>]) -> String {
    format!(
        r#"
The following goals have conflicting reporting period labels:

{conflicts}

Unlabeled goals use the default period of 4 weeks.
"#,
        conflicts = conflicts
            .iter()
            .map(|e| format!("+ {}: {}", e.goal.link(), e.reason))
            .join("\n")
    )
}

fn error_sections(errors: &ReminderErrors<'_>) -> String {
    let mut sections = Vec::new();

    if !errors.unowned.is_empty() {
        sections.push(unowned_errors(&errors.unowned));
    }
    if errors.ownership.iter().any(|p| p.owners.has_multiple()) {
        sections.push(multiple_owner_warnings(&errors.ownership));
    }
    if errors
        .ownership
        .iter()
        .any(|p| p.owners.has_missing_zulip_id())
    {
        sections.push(missing_zulip_errors(&errors.ownership));
    }
    if errors
        .ownership
        .iter()
        .any(|p| p.owners.has_missing_team_entry())
    {
        sections.push(missing_team_entry_warnings(&errors.ownership));
    }
    if !errors.schedule.is_empty() {
        sections.push(schedule_warnings(&errors.schedule));
    }

    sections.iter().join("\n\n---\n\n")
}

async fn build_plan<'gh>(
    issues: &'gh [GoalIssue],
    team: &TeamClient,
    now: DateTime<Utc>,
) -> anyhow::Result<ReminderPlan<'gh>> {
    let mut plan = ReminderPlan::default();

    for issue in issues {
        let (reminder, conflict) = Reminder::from_issue(issue, now);

        log::debug!(
            "issue #{}: period_start = {}, next_deadline = {}, last_comment = {:?}",
            issue.number,
            reminder.period_start,
            reminder.next_period_start,
            issue.last_comment.as_ref().map(|c| c.created_at),
        );

        if let Some(conflict) = conflict {
            plan.add_conflicts(reminder.goal, conflict);
        }

        if !reminder.is_required(now) {
            continue;
        }

        let owners = Owners::resolve_goal(team, issue).await?;
        plan.add_goal(reminder, owners);
    }

    Ok(plan)
}

async fn send_dm(zulip: &ZulipClient, owner: ZulipId, content: &str, dry_run: bool) {
    if dry_run {
        log::debug!("(DRY) Would send DM to user {}: {}", owner.0, content);
        return;
    }

    let req = MessageApiRequest {
        recipient: Recipient::Private {
            id: owner.0,
            email: "",
        },
        content,
    };

    if let Err(err) = req.send(zulip).await {
        log::error!("failed to send a DM on Zulip: {err}")
    }
}

async fn send_triagebot_topic(
    zulip: &ZulipClient,
    content: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    if dry_run {
        log::debug!(
            "(DRY) Would send to topic {GOALS_META_STREAM}>{TRIAGEBOT_TOPIC}: {}",
            content,
        );
        return Ok(());
    }

    MessageApiRequest {
        recipient: Recipient::Stream {
            id: GOALS_META_STREAM,
            topic: TRIAGEBOT_TOPIC,
        },
        content,
    }
    .send(zulip)
    .await?;

    Ok(())
}

#[derive(Default)]
struct PeriodCounts {
    weekly: usize,
    every_2_weeks: usize,
    every_4_weeks: usize,
}

impl PeriodCounts {
    fn from_reminders<'gh>(reminders: impl Iterator<Item = Reminder<'gh>>) -> Self {
        let mut counts = Self::default();

        for reminder in reminders {
            match reminder.period {
                Period::EveryWeek => counts.weekly += 1,
                Period::Every2Weeks => counts.every_2_weeks += 1,
                Period::Every4Weeks => counts.every_4_weeks += 1,
            }
        }

        counts
    }

    fn total(&self) -> usize {
        self.weekly + self.every_2_weeks + self.every_4_weeks
    }
}

fn report(
    errors: &ReminderErrors<'_>,
    total_owners: usize,
    counts: &PeriodCounts,
    today: NaiveDate,
) -> String {
    let next_week = Period::EveryWeek.next_start(Period::EveryWeek.start(today));
    let next_2_weeks = Period::Every2Weeks.next_start(Period::Every2Weeks.start(today));
    let next_4_weeks = Period::Every4Weeks.next_start(Period::Every4Weeks.start(today));

    let error_summary = if errors.is_empty() {
        "No errors happened in the process.".to_owned()
    } else {
        format!(
            "{count} errors happened in the process.\n\n---\n\n{details}",
            count = errors.count(),
            details = error_sections(errors),
        )
    };

    format!(
        r#"
Hi @*T-goals*!

Weekly run finished.

{total_owners} owners were notified about {total_goals} goals:
+ Weekly reports: {weekly} (next cycle: {next_week})
+ Biweekly reports: {every_2_weeks} (next cycle: {next_2_weeks})
+ Four-week reports: {every_4_weeks} (next cycle: {next_4_weeks})

{error_summary}

Until next week! <3
"#,
        total_goals = counts.total(),
        weekly = counts.weekly,
        every_2_weeks = counts.every_2_weeks,
        every_4_weeks = counts.every_4_weeks,
        next_week = display_job_date(next_week),
        next_2_weeks = display_job_date(next_2_weeks),
        next_4_weeks = display_job_date(next_4_weeks),
    )
}

async fn execute_plan(
    zulip: &ZulipClient,
    plan: ReminderPlan<'_>,
    today: NaiveDate,
    dry_run: bool,
) -> anyhow::Result<()> {
    let ReminderPlan {
        goals_by_owner,
        errors,
    } = plan;

    let total_owners = goals_by_owner.len();
    let counts = PeriodCounts::from_reminders(
        goals_by_owner
            .values()
            .flatten()
            .copied()
            .unique_by(|reminder| reminder.goal.issue),
    );

    for (owner, goals) in goals_by_owner {
        send_dm(zulip, owner, &owner_message(owner, &goals), dry_run).await;
    }

    send_triagebot_topic(
        zulip,
        &report(&errors, total_owners, &counts, today),
        dry_run,
    )
    .await?;

    Ok(())
}

pub async fn ping_project_goals_owners(
    gh: &GithubClient,
    zulip: &ZulipClient,
    team: &TeamClient,
    dry_run: bool,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let issues = gh.open_goal_issues().await?;
    let plan = build_plan(&issues, team, now).await?;
    execute_plan(zulip, plan, now.date_naive(), dry_run).await
}

pub struct PingProjectGoalsOwnersJob;

#[async_trait]
impl Job for PingProjectGoalsOwnersJob {
    fn name(&self) -> &'static str {
        "ping_project_goal_owners_job"
    }

    async fn run(&self, ctx: &Context, _metadata: &serde_json::Value) -> anyhow::Result<()> {
        ping_project_goals_owners(&ctx.github, &ctx.zulip, &ctx.team, false).await
    }
}

/// Returns true if the GitHub user is part of the Goals team.
pub async fn is_goals_member(team_client: &TeamClient, github_id: u64) -> anyhow::Result<bool> {
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

fn is_tracking_issue(issue: &Issue) -> bool {
    issue
        .labels
        .iter()
        .any(|label| label.name == "C-tracking-issue")
}

async fn create_goal_topic(issue: &Issue, ctx: &Context) -> anyhow::Result<()> {
    if !is_tracking_issue(issue) {
        return Ok(());
    }

    let owners = Owners::resolve_event(&ctx.team, issue).await?;
    let topic = goal_zulip_topic(issue);
    let content = format!(
        "Goal *{title}* (goals#{number}) has been accepted. It's owned by {owners}.",
        title = issue.title,
        number = issue.number,
        owners = owners
            .all_mentions(false)
            .unwrap_or_else(|| "nobody (@*T-goals* should fix this)".to_owned()),
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

    let author =
        Owner::from_id_and_username(&ctx.team, comment.user.id, &comment.user.login).await?;
    let text = &comment.body;

    let content = format!(
        "[Comment posted]({url}) on goals#{number} by {author}:\n\
         {ticks}quote\n\
         {text}\n\
         {ticks}",
        url = comment.html_url,
        number = issue.number,
        author = author.display_mention(true),
        ticks = quote_fence(text),
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
