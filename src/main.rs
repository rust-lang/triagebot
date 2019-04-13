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

mod config;
mod github;
mod handlers;
mod interactions;
mod payload;
mod team;

use interactions::ErrorComment;
use payload::SignedPayload;

enum EventName {
    IssueComment,
    Other,
}

impl<'a, 'r> request::FromRequest<'a, 'r> for EventName {
    type Error = String;
    fn from_request(req: &'a Request<'r>) -> request::Outcome<Self, Self::Error> {
        let ev = if let Some(ev) = req.headers().get_one("X-GitHub-Event") {
            ev
        } else {
            return Outcome::Failure((Status::BadRequest, "Needs a X-GitHub-Event".into()));
        };
        let ev = match ev {
            "issue_comment" => EventName::IssueComment,
            _ => EventName::Other,
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
    event: EventName,
    payload: SignedPayload,
    ctx: State<handlers::Context>,
) -> Result<(), WebhookError> {
    match event {
        EventName::IssueComment => {
            let payload = payload
                .deserialize::<github::IssueCommentEvent>()
                .context("IssueCommentEvent failed to deserialize")
                .map_err(Error::from)?;

            let event = github::Event::IssueComment(payload);
            if let Err(err) = handlers::handle(&ctx, &event) {
                if let Some(issue) = event.issue() {
                    ErrorComment::new(issue, err.to_string()).post(&ctx.github)?;
                }
                return Err(err.into());
            }
        }
        // Other events need not be handled
        EventName::Other => {}
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
    let gh = github::GithubClient::new(
        client.clone(),
        env::var("GITHUB_API_TOKEN").expect("Missing GITHUB_API_TOKEN"),
    );
    let ctx = handlers::Context {
        github: gh.clone(),
        username: github::User::current(&gh).unwrap().login,
    };

    let mut config = rocket::Config::active().unwrap();
    config.set_port(
        env::var("TRIAGEBOT_PORT")
            .map(|port| port.parse().unwrap())
            .unwrap_or(8000),
    );
    rocket::custom(config)
        .manage(gh)
        .manage(ctx)
        .mount("/", routes![webhook])
        .register(catchers![not_found])
        .launch();
}
