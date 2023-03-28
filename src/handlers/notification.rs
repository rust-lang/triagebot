//! Purpose: Allow any user to ping a pre-selected group of people on GitHub via comments.
//!
//! The set of "teams" which can be pinged is intentionally restricted via configuration.
//!
//! Parsing is done in the `parser::command::ping` module.

use crate::{
    db::notifications::Notification,
    github::{self, Event},
    handlers::Context,
};
use anyhow::Context as _;
use std::collections::HashSet;
use std::convert::{TryFrom, TryInto};
use tracing as log;

pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let body = match event.comment_body() {
        Some(v) => v,
        // Skip events that don't have comment bodies associated
        None => return Ok(()),
    };

    if let Event::Issue(e) = event {
        if !matches!(
            e.action,
            github::IssuesAction::Opened | github::IssuesAction::Edited
        ) {
            // no change in issue's body for these events, so skip
            return Ok(());
        }
    }

    let short_description = match event {
        Event::Issue(e) => e.issue.title.clone(),
        Event::IssueComment(e) => format!("Comment on {}", e.issue.title),
        Event::Push(_) | Event::Create(_) => return Ok(()),
    };

    let mut caps = parser::get_mentions(body)
        .into_iter()
        .collect::<HashSet<_>>();

    // FIXME: Remove this hardcoding. Ideally we need organization-wide
    // configuration, but it's unclear where to put it.
    if event.issue().unwrap().repository().organization == "serde-rs" {
        // Only add dtolnay on new issues/PRs, not on comments to old PRs and
        // issues.
        if let Event::Issue(e) = event {
            if e.action == github::IssuesAction::Opened {
                caps.insert("dtolnay");
            }
        }
    }

    // Get the list of users already notified by a previous version of this
    // comment, so they don't get notified again
    let mut users_notified = HashSet::new();
    if let Some(from) = event.comment_from() {
        for login in parser::get_mentions(from).into_iter() {
            if let Some((Ok(users), _)) = id_from_user(ctx, login).await? {
                users_notified.extend(users.into_iter().map(|user| user.id.unwrap()));
            }
        }
    };

    // We've implicitly notified the user that is submitting the notification:
    // they already know that they left this comment.
    //
    // If the user intended to ping themselves, they can add the GitHub comment
    // via the Zulip interface.
    match event.user().get_id(&ctx.github).await {
        Ok(Some(id)) => {
            users_notified.insert(id.try_into().unwrap());
        }
        Ok(None) => {}
        Err(err) => {
            log::error!("Failed to query ID for {:?}: {:?}", event.user(), err);
        }
    }
    log::trace!("Captured usernames in comment: {:?}", caps);
    for login in caps {
        let (users, team_name) = match id_from_user(ctx, login).await? {
            Some((users, team_name)) => (users, team_name),
            None => continue,
        };

        let users = match users {
            Ok(users) => users,
            Err(err) => {
                log::error!("getting users failed: {:?}", err);
                continue;
            }
        };

        let mut connection = ctx.db.connection().await;
        for user in users {
            if !users_notified.insert(user.id.unwrap()) {
                // Skip users already associated with this event.
                continue;
            }

            if let Err(err) = connection
                .record_username(user.id.unwrap(), user.login)
                .await
                .context("failed to record username")
            {
                log::error!("record username: {:?}", err);
            }

            if let Err(err) = connection
                .record_ping(&Notification {
                    user_id: user.id.unwrap(),
                    origin_url: event.html_url().unwrap().to_owned(),
                    origin_html: body.to_owned(),
                    time: event.time().unwrap(),
                    short_description: Some(short_description.clone()),
                    team_name: team_name.clone(),
                })
                .await
                .context("failed to record ping")
            {
                log::error!("record ping: {:?}", err);
            }
        }
    }

    Ok(())
}

async fn id_from_user(
    ctx: &Context,
    login: &str,
) -> anyhow::Result<Option<(anyhow::Result<Vec<github::User>>, Option<String>)>> {
    if login.contains('/') {
        // This is a team ping. For now, just add it to everyone's agenda on
        // that team, but also mark it as such (i.e., a team ping) for
        // potentially different prioritization and so forth.
        //
        // In order to properly handle this down the road, we will want to
        // distinguish between "everyone must pay attention" and "someone
        // needs to take a look."
        //
        // We may also want to be able to categorize into these buckets
        // *after* the ping occurs and is initially processed.

        let mut iter = login.split('/');
        let _rust_lang = iter.next().unwrap();
        let team = iter.next().unwrap();
        let team = match github::get_team(&ctx.github, team).await {
            Ok(Some(team)) => team,
            Ok(None) => {
                // If the team is in rust-lang*, then this is probably an error (potentially user
                // error, but should be investigated). Otherwise it's probably not going to be in
                // the team repository so isn't actually an error.
                if login.starts_with("rust") {
                    log::error!("team ping ({}) failed to resolve to a known team", login);
                } else {
                    log::info!("team ping ({}) failed to resolve to a known team", login);
                }
                return Ok(None);
            }
            Err(err) => {
                log::error!(
                    "team ping ({}) failed to resolve to a known team: {:?}",
                    login,
                    err
                );
                return Ok(None);
            }
        };

        Ok(Some((
            team.members
                .into_iter()
                .map(|member| {
                    let id = i64::try_from(member.github_id)
                        .with_context(|| format!("user id {} out of bounds", member.github_id))?;
                    Ok(github::User {
                        id: Some(id),
                        login: member.github,
                    })
                })
                .collect::<anyhow::Result<Vec<github::User>>>(),
            Some(team.name),
        )))
    } else {
        let user = github::User {
            login: login.to_owned(),
            id: None,
        };
        let id = user
            .get_id(&ctx.github)
            .await
            .with_context(|| format!("failed to get user {} ID", user.login))?;
        let id = match id {
            Some(id) => id,
            // If the user was not in the team(s) then just don't record it.
            None => {
                log::trace!("Skipping {} because no id found", user.login);
                return Ok(None);
            }
        };
        let id = i64::try_from(id).with_context(|| format!("user id {} out of bounds", id));
        Ok(Some((
            id.map(|id| {
                vec![github::User {
                    login: user.login.clone(),
                    id: Some(id),
                }]
            }),
            None,
        )))
    }
}
