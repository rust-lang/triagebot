#![feature(proc_macro_hygiene, decl_macro)]
#![allow(clippy::new_without_default)]

#[macro_use]
extern crate rocket;

use failure::{Error, ResultExt};
use reqwest::Client;
use rocket::request;
use rocket::State;
use rocket::{http::Status, Outcome, Request};
use std::env;
use std::sync::Arc;

mod handlers;
mod registry;

mod config;
mod github;
mod interactions;
mod payload;
mod team;

use github::{Comment, GithubClient, Issue, User};
use payload::SignedPayload;
use registry::HandleRegistry;

#[derive(PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueCommentAction {
    Created,
    Edited,
    Deleted,
}

#[derive(Debug, serde::Deserialize)]
pub struct IssueCommentEvent {
    action: IssueCommentAction,
    issue: Issue,
    comment: Comment,
    repository: Repository,
}

#[derive(Debug, serde::Deserialize)]
pub struct Repository {
    full_name: String,
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
            return Outcome::Failure((Status::BadRequest, "Needs a X-GitHub-Event".into()));
        };
        let ev = match ev {
            "issue_comment" => Event::IssueComment,
            _ => Event::Other,
        };
        Outcome::Success(ev)
    }
}

#[derive(Debug)]
struct WebhookError(Error);

impl<'r> rocket::response::Responder<'r> for WebhookError {
    fn respond_to(self, _: &Request) -> rocket::response::Result<'r> {
        let body = format!("{:?}", self.0);
        rocket::Response::build()
            .header(rocket::http::ContentType::Plain)
            .status(rocket::http::Status::InternalServerError)
            .sized_body(std::io::Cursor::new(body))
            .ok()
    }
}

impl From<Error> for WebhookError {
    fn from(e: Error) -> WebhookError {
        WebhookError(e)
    }
}

#[post("/github-hook", data = "<payload>")]
fn webhook(
    event: Event,
    payload: SignedPayload,
    reg: State<HandleRegistry>,
) -> Result<(), WebhookError> {
    match event {
        Event::IssueComment => {
            let payload = payload
                .deserialize::<IssueCommentEvent>()
                .context("IssueCommentEvent failed to deserialize")
                .map_err(Error::from)?;

            let event = registry::Event::IssueComment(payload);
            reg.handle(&event).map_err(Error::from)?;
        }
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
    let gh = GithubClient::new(
        client.clone(),
        env::var("GITHUB_API_TOKEN").expect("Missing GITHUB_API_TOKEN"),
    );
    let username = Arc::new(User::current(&gh).unwrap().login);
    let mut registry = HandleRegistry::new();
    handlers::register_all(&mut registry, gh.clone(), username);

    let mut config = rocket::Config::active().unwrap();
    config.set_port(
        env::var("TRIAGEBOT_PORT")
            .map(|port| port.parse().unwrap())
            .unwrap_or(8000),
    );
    rocket::custom(config)
        .manage(gh)
        .manage(registry)
        .mount("/", routes![webhook])
        .register(catchers![not_found])
        .launch();
}
