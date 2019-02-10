#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;

use failure::Error;
use lazy_static::lazy_static;
use openssl::hash::MessageDigest;
use openssl::memcmp;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use regex::Regex;
use reqwest::Client;
use rocket::data::{self, FromDataSimple};
use rocket::request;
use rocket::State;
use rocket::{http::Status, Data, Outcome, Request};
use std::env;
use std::io::Read;

mod github;
mod permissions;

static BOT_USER_NAME: &str = "rust-highfive";

use github::{Comment, GithubClient, Issue, Label};
use permissions::{Permissions, Team};

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueCommentAction {
    Created,
    Edited,
    Deleted,
}

#[derive(Debug, serde::Deserialize)]
struct IssueCommentEvent {
    action: IssueCommentAction,
    issue: Issue,
    comment: Comment,
}

impl IssueCommentEvent {
    /// Purpose: Allow any user to modify issue labels on GitHub via comments.
    ///
    /// The current syntax allows adding labels (+labelname or just labelname) following the
    /// `label:` prefix. Users can also remove labels with -labelname.
    ///
    /// No verification is currently attempted of the added labels (only currently present labels
    /// can be removed). XXX: How does this affect users workflow?
    ///
    /// There will be no feedback beyond the label change to reduce notification noise.
    fn handle_labels(&mut self, g: &GithubClient) -> Result<(), Error> {
        lazy_static! {
            static ref LABEL_RE: Regex = Regex::new(r#"\blabel: (\S+\s*)+"#).unwrap();
        }

        let mut issue_labels = self.issue.labels().to_owned();

        for label_block in LABEL_RE.find_iter(&self.comment.body) {
            let label_block = &label_block.as_str()["label: ".len()..]; // guaranteed to start with this
            for label in label_block.split_whitespace() {
                if label.starts_with('-') {
                    if let Some(label) = issue_labels.iter().position(|el| el.name == &label[1..]) {
                        issue_labels.remove(label);
                    } else {
                        // do nothing, if the user attempts to remove a label that's not currently
                        // set simply skip it
                    }
                } else if label.starts_with('+') {
                    // add this label, but without the +
                    issue_labels.push(Label {
                        name: label[1..].to_string(),
                    });
                } else {
                    // add this label (literally)
                    issue_labels.push(Label {
                        name: label.to_string(),
                    });
                }
            }
        }

        self.issue.set_labels(&g, issue_labels)
    }

    /// Permit assignment of any user to issues, without requiring "write" access to the repository.
    ///
    /// It is unknown which approach is needed here: we may need to fake-assign ourselves and add a
    /// 'claimed by' section to the top-level comment. That would be very unideal.
    ///
    /// The ideal workflow here is that the user is added to a read-only team with no access to the
    /// repository and immediately thereafter assigned to the issue.
    ///
    /// Such assigned issues should also be placed in a queue to ensure that the user remains
    /// active; the assigned user will be asked for a status report every 2 weeks (XXX: timing).
    ///
    /// If we're intending to ask for a status report but no comments from the assigned user have
    /// been given for the past 2 weeks, the bot will de-assign the user. They can once more claim
    /// the issue if necessary.
    ///
    /// Assign users with `assign: @gh-user` or `@bot claim` (self-claim).
    fn handle_assign(&mut self, g: &GithubClient) -> Result<(), Error> {
        lazy_static! {
            static ref RE_ASSIGN: Regex = Regex::new(r"\bassign: @(\S+)").unwrap();
            static ref RE_CLAIM: Regex =
                Regex::new(&format!(r"\b@{} claim\b", BOT_USER_NAME)).unwrap();
        }

        // XXX: Handle updates to the comment specially to avoid double-queueing or double-assigning
        // and other edge cases.

        if RE_CLAIM.is_match(&self.comment.body) {
            self.issue.add_assignee(g, &self.comment.user.login)?;
        } else {
            if let Some(capture) = RE_ASSIGN.captures(&self.comment.body) {
                self.issue.add_assignee(g, &capture[1])?;
            }
        }

        // TODO: Enqueue a check-in in two weeks.
        // TODO: Post a comment documenting the biweekly check-in? Maybe just give them two weeks
        //       without any commentary from us.
        // TODO: How do we handle `claim`/`assign:` if someone's already assigned? Error out?

        Ok(())
    }

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
    fn handle_create_tracking_issue(
        &mut self,
        g: &GithubClient,
        auth: &Permissions,
    ) -> Result<(), Error> {
        lazy_static! {
            static ref RE_TRACKING: Regex = Regex::new(&format!(
                r#"\b@{} tracking-issue create feature=("[^"]+|\S+) team=(libs|lang)"#,
                BOT_USER_NAME,
            ))
            .unwrap();
        }

        // Skip this event if the comment is edited or deleted.
        if self.action != IssueCommentAction::Created {
            return Ok(());
        }

        let feature;
        let team;

        if let Some(captures) = RE_TRACKING.captures(&self.comment.body) {
            feature = captures.get(1).unwrap();
            team = captures.get(2).unwrap().as_str().parse::<Team>()?;
        } else {
            // no tracking issue creation comment
            return Ok(());
        }

        // * Create tracking issue (C-tracking-issue, T-{team})
        // * Post comment with link to issue and suggestion on what to do next

        unimplemented!()
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
    fn handle_link_tracking_issue(&mut self, g: &GithubClient) -> Result<(), Error> {
        unimplemented!()
    }

    fn run(mut self, g: &GithubClient, permissions: &Permissions) -> Result<(), Error> {
        // Don't do anything on deleted comments.
        //
        // XXX: Should we attempt to roll back the action instead?
        if self.action == IssueCommentAction::Deleted {
            return Ok(());
        }
        self.handle_labels(g)?;
        Ok(())
    }
}

enum Event {
    IssueComment,
    Other,
}

impl<'a, 'r> request::FromRequest<'a, 'r> for Event {
    type Error = String;
    fn from_request(req: &'a Request<'r>) -> request::Outcome<Self, Self::Error> {
        let ev = if let Some(ev) = req.headers().get_one("X-GitHub-Event") {
            ev
        } else {
            return Outcome::Failure((Status::BadRequest, format!("Needs a X-GitHub-Event")));
        };
        let ev = match ev {
            "issue_comment" => Event::IssueComment,
            _ => Event::Other,
        };
        Outcome::Success(ev)
    }
}

struct SignedPayload(Vec<u8>);

impl FromDataSimple for SignedPayload {
    type Error = String;
    fn from_data(req: &Request, data: Data) -> data::Outcome<Self, Self::Error> {
        let signature = match req.headers().get_one("X-Hub-Signature") {
            Some(s) => s,
            None => {
                return Outcome::Failure((
                    Status::Unauthorized,
                    format!("Unauthorized, no signature"),
                ));
            }
        };
        let signature = &signature["sha1=".len()..];
        let signature = match hex::decode(&signature) {
            Ok(e) => e,
            Err(e) => {
                return Outcome::Failure((
                    Status::BadRequest,
                    format!(
                        "failed to convert signature {:?} from hex: {:?}",
                        signature, e
                    ),
                ));
            }
        };

        let mut stream = data.open().take(1024 * 1024 * 5); // 5 Megabytes
        let mut buf = Vec::new();
        if let Err(err) = stream.read_to_end(&mut buf) {
            return Outcome::Failure((
                Status::InternalServerError,
                format!("failed to read request body to string: {:?}", err),
            ));
        }

        let key = PKey::hmac(env::var("GITHUB_WEBHOOK_SECRET").unwrap().as_bytes()).unwrap();
        let mut signer = Signer::new(MessageDigest::sha1(), &key).unwrap();
        signer.update(&buf).unwrap();
        let hmac = signer.sign_to_vec().unwrap();

        if !memcmp::eq(&hmac, &signature) {
            return Outcome::Failure((Status::Unauthorized, format!("HMAC not correct")));
        }

        Outcome::Success(SignedPayload(buf))
    }
}

impl SignedPayload {
    fn deserialize<T: serde::de::DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.0)
    }
}

#[post("/github-hook", data = "<payload>")]
fn webhook(
    event: Event,
    payload: SignedPayload,
    client: State<GithubClient>,
    permissions: State<Permissions>,
) -> Result<(), String> {
    match event {
        Event::IssueComment => payload
            .deserialize::<IssueCommentEvent>()
            .map_err(|e| format!("IssueCommentEvent failed to deserialize: {:?}", e))?
            .run(&client, &permissions)
            .map_err(|e| e.to_string())?,
        // Other events need not be handled
        Event::Other => {}
    }
    Ok(())
}

#[catch(404)]
fn not_found(_: &Request) -> &'static str {
    "Not Found"
}

fn main() {
    dotenv::dotenv().ok();
    let client = Client::new();
    rocket::ignite()
        .manage(GithubClient::new(
            client.clone(),
            env::var("GITHUB_API_TOKEN").unwrap(),
        ))
        .manage(Permissions::new(client))
        .mount("/", routes![webhook])
        .register(catchers![not_found])
        .launch();
}
