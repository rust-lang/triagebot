use crate::{
    github::GithubClient,
    registry::{Event, Handler},
    team::Team,
    IssueCommentAction, IssueCommentEvent,
};
use failure::Error;
use lazy_static::lazy_static;
use regex::Regex;

pub struct TrackingIssueHandler {
    pub client: GithubClient,
}

impl TrackingIssueHandler {
    /// Automates creating tracking issues.
    ///
    /// This command is initially restricted to members of Rust teams.
    ///
    /// This command is rare, and somewhat high-impact, so it requires the `@bot` prefix.
    /// The syntax for creating a tracking issue follows. Note that only the libs and lang teams are
    /// currently supported; it's presumed that the other teams may want significantly different
    /// issue formats, so only these two are supported for the time being.
    ///
    /// `@bot tracking-issue create feature="<short feature description>" team=[libs|lang]`
    ///
    /// This creates the tracking issue, though it's likely that the invokee will want to edit its
    /// body/title.
    ///
    /// Long-term, this will also create a thread on internals and lock the tracking issue,
    /// directing commentary to the thread, but for the time being we limit the scope of work as
    /// well as project impact.
    fn handle_create(&self, event: &IssueCommentEvent) -> Result<(), Error> {
        lazy_static! {
            static ref RE_TRACKING: Regex = Regex::new(&format!(
                r#"\b@{} tracking-issue create feature=("[^"]+|\S+) team=(libs|lang)"#,
                crate::BOT_USER_NAME,
            ))
            .unwrap();
        }

        // Skip this event if the comment is edited or deleted.
        if event.action != IssueCommentAction::Created {
            return Ok(());
        }

        #[allow(unused)]
        let feature;
        #[allow(unused)]
        let team;

        if let Some(captures) = RE_TRACKING.captures(&event.comment.body) {
            #[allow(unused)]
            {
                feature = captures.get(1).unwrap();
                team = captures.get(2).unwrap().as_str().parse::<Team>()?;
            }
        } else {
            // no tracking issue creation comment
            return Ok(());
        }

        // * Create tracking issue (C-tracking-issue, T-{team})
        // * Post comment with link to issue and suggestion on what to do next

        Ok(())
    }

    /// Links issues to tracking issues.
    ///
    /// We verify that the tracking issue listed is in fact a tracking issue (i.e., has the
    /// C-tracking-issue label). Next, the tracking issue's top comment is updated with a link and
    /// title of the issue linked as a checkbox in the bugs list.
    ///
    /// We also label the issue with `tracked-bug`.
    ///
    /// TODO: Check the checkbox in the tracking issue when `tracked-bug` is closed.
    ///
    /// Syntax: `link: #xxx`
    fn handle_link(&self, _event: &IssueCommentEvent) -> Result<(), Error> {
        Ok(())
    }
}

impl Handler for TrackingIssueHandler {
    fn handle_event(&self, event: &Event) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(());
        };

        self.handle_create(&event)?;
        self.handle_link(&event)?;
        Ok(())
    }
}
