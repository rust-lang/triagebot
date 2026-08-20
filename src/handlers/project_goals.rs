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

const RUST_PROJECT_GOALS_REPO: &str = "rust-lang/goals";
const GOALS_TEAM: &str = "goals";

/// Give new goals a grace period before reminders begin.
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

    /// Returns a string representing an issue in the `rust-lang/goals` repo.
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

    fn is_in_grace_period(&self, now: DateTime<Utc>) -> bool {
        now < self.goal.created_at + Duration::days(FIRST_REPORT_GRACE_DAYS)
    }

    fn has_current_update(&self) -> bool {
        self.goal
            .last_comment_at
            .is_some_and(|d| d.date_naive() >= self.period_start)
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
    counters: Counters,
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

A goal should have exactly one owner. Notification was attempted for all owners with a Zulip account.
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
                        Some(owners) => format!("Notification was attempted for {owners}."),
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
                        Some(owners) => format!("Notification was attempted for {owners}."),
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

fn failed_dm_errors(failed_dms: &[(ZulipId, String, String)]) -> String {
    format!(
        r#"
These DMs could not be sent:

{failed_dms}
"#,
        failed_dms = failed_dms
            .iter()
            .map(|(owner, goals, error)| {
                format!("+ {owner} ({goals}): {error}", owner = owner.mention(true))
            })
            .join("\n")
    )
}

fn error_sections(errors: &ReminderErrors<'_>, failed_dms: &[(ZulipId, String, String)]) -> String {
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
    if !failed_dms.is_empty() {
        sections.push(failed_dm_errors(failed_dms));
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

        if plan.counters.count(&reminder, now) {
            let owners = Owners::resolve_goal(team, issue).await?;
            plan.add_goal(reminder, owners);
        }
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
        log::debug!("(DRY) Would send DM to user {}: {}", owner.0, content);
        return Ok(());
    }

    MessageApiRequest {
        recipient: Recipient::DirectMessage { id: owner.0 },
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
struct PeriodCounter {
    due: usize,
    updated: usize,
    graced: usize,
}

impl PeriodCounter {
    fn total(&self) -> usize {
        self.due + self.updated + self.graced
    }
}

#[derive(Default)]
struct Counters {
    weekly: PeriodCounter,
    biweekly: PeriodCounter,
    four_week: PeriodCounter,
}

impl Counters {
    /// Count the goal and return whether it is due for an update.
    fn count(&mut self, reminder: &Reminder<'_>, now: DateTime<Utc>) -> bool {
        let period = match reminder.period {
            Period::EveryWeek => &mut self.weekly,
            Period::Every2Weeks => &mut self.biweekly,
            Period::Every4Weeks => &mut self.four_week,
        };

        if reminder.is_in_grace_period(now) {
            period.graced += 1;
            false
        } else if reminder.has_current_update() {
            period.updated += 1;
            false
        } else {
            period.due += 1;
            true
        }
    }

    fn total(&self) -> usize {
        self.weekly.total() + self.biweekly.total() + self.four_week.total()
    }
}

fn report(
    errors: &ReminderErrors<'_>,
    failed_dms: &[(ZulipId, String, String)],
    total_dms: usize,
    reminded_goals: usize,
    counters: &Counters,
    today: NaiveDate,
) -> String {
    let next_week = Period::EveryWeek.next_start(Period::EveryWeek.start(today));
    let next_2_weeks = Period::Every2Weeks.next_start(Period::Every2Weeks.start(today));
    let next_4_weeks = Period::Every4Weeks.next_start(Period::Every4Weeks.start(today));

    let error_summary = if errors.is_empty() && failed_dms.is_empty() {
        "No errors happened in the process.".to_owned()
    } else {
        let error_count = errors.count() + failed_dms.len();
        format!(
            "{error_count} errors happened in the process.\n\n---\n\n{details}",
            details = error_sections(errors, failed_dms),
        )
    };
    let ok_dms = total_dms - failed_dms.len();

    format!(
        r#"
Hi @*T-goals*!

Weekly run finished.

There are {total} open goals:
+ Weekly reports: {weekly_due} due, {weekly_updated} updated, {weekly_graced} in grace period
  - Next cycle starts {next_week}
+ Biweekly reports: {biweekly_due} due, {biweekly_updated} updated, {biweekly_graced} in grace period
  - Next cycle starts {next_2_weeks}
+ Four-week reports: {four_week_due} due, {four_week_updated} updated, {four_week_graced} in grace period
  - Next cycle starts {next_4_weeks}

Reminders were prepared for {total_dms} owners about {reminded_goals} goals.

{ok_dms} of {total_dms} reminders were sent.

{error_summary}

Until next week! <3
"#,
        total = counters.total(),
        weekly_due = counters.weekly.due,
        weekly_updated = counters.weekly.updated,
        weekly_graced = counters.weekly.graced,
        biweekly_due = counters.biweekly.due,
        biweekly_updated = counters.biweekly.updated,
        biweekly_graced = counters.biweekly.graced,
        four_week_due = counters.four_week.due,
        four_week_updated = counters.four_week.updated,
        four_week_graced = counters.four_week.graced,
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
        counters,
    } = plan;

    let total_owners = goals_by_owner.len();
    let reminded_goals = goals_by_owner
        .values()
        .flatten()
        .unique_by(|reminder| reminder.goal.issue)
        .count();

    let mut failed_dms = Vec::new();
    for (owner, goals) in goals_by_owner {
        if let Err(error) = send_dm(zulip, owner, &owner_message(owner, &goals), dry_run).await {
            log::error!("failed to send a DM to Zulip user {}: {error}", owner.0);
            let goals = goals.iter().map(|reminder| reminder.goal.link()).join(", ");
            failed_dms.push((owner, goals, error.to_string()));
        }
    }

    send_triagebot_topic(
        zulip,
        &report(
            &errors,
            &failed_dms,
            total_owners,
            reminded_goals,
            &counters,
            today,
        ),
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

    let posted = MessageApiRequest {
        recipient: Recipient::Stream {
            id: GOALS_STREAM,
            topic: &goal_zulip_topic(issue),
        },
        content: &content,
    }
    .send(&ctx.zulip)
    .await?;

    // Add a reaction (:book:) so it's easier to acknowledge the update.
    ctx.zulip.add_reaction(posted.message_id, "book").await?;

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
