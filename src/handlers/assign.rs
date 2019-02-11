//! Permit assignment of any user to issues, without requiring "write" access to the repository.
//!
//! It is unknown which approach is needed here: we may need to fake-assign ourselves and add a
//! 'claimed by' section to the top-level comment. That would be very unideal.
//!
//! The ideal workflow here is that the user is added to a read-only team with no access to the
//! repository and immediately thereafter assigned to the issue.
//!
//! Such assigned issues should also be placed in a queue to ensure that the user remains
//! active; the assigned user will be asked for a status report every 2 weeks (XXX: timing).
//!
//! If we're intending to ask for a status report but no comments from the assigned user have
//! been given for the past 2 weeks, the bot will de-assign the user. They can once more claim
//! the issue if necessary.
//!
//! Assign users with `/assign @gh-user` or `/claim` (self-claim).

use crate::{
    github::GithubClient,
    registry::{Event, Handler},
};
use failure::Error;
use lazy_static::lazy_static;
use regex::Regex;

pub struct AssignmentHandler {
    pub client: GithubClient,
}

impl Handler for AssignmentHandler {
    fn handle_event(&self, event: &Event) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(());
        };

        lazy_static! {
            static ref RE_ASSIGN: Regex = Regex::new(r"\b/assign @(\S+)").unwrap();
            static ref RE_CLAIM: Regex = Regex::new(r"\b/claim\b").unwrap();
        }

        if RE_CLAIM.is_match(&event.comment.body) {
            event
                .issue
                .add_assignee(&self.client, &event.comment.user.login)?;
        } else {
            if let Some(capture) = RE_ASSIGN.captures(&event.comment.body) {
                event.issue.add_assignee(&self.client, &capture[1])?;
            }
        }

        // TODO: Enqueue a check-in in two weeks.
        // TODO: Post a comment documenting the biweekly check-in? Maybe just give them two weeks
        //       without any commentary from us.

        Ok(())
    }
}
