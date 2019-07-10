#![feature(proc_macro_hygiene, decl_macro)]
#![allow(clippy::new_without_default)]

#[macro_use]
extern crate rocket;

use failure::ResultExt;
use reqwest::Client;
use rocket::{
    data::Data,
    http::Status,
    request::{self, FromRequest, Request},
    Outcome, State,
};
use std::{env, io::Read};
use triagebot::{github, handlers, payload, EventName, WebhookError};

struct XGitHubEvent<'r>(&'r str);

impl<'a, 'r> FromRequest<'a, 'r> for XGitHubEvent<'a> {
    type Error = &'static str;
    fn from_request(req: &'a Request<'r>) -> request::Outcome<Self, Self::Error> {
        let ev = if let Some(ev) = req.headers().get_one("X-GitHub-Event") {
            ev
        } else {
            return Outcome::Failure((Status::BadRequest, "Needs a X-GitHub-Event"));
        };
        Outcome::Success(XGitHubEvent(ev))
    }
}

struct XHubSignature<'r>(&'r str);

impl<'a, 'r> FromRequest<'a, 'r> for XHubSignature<'a> {
    type Error = &'static str;
    fn from_request(req: &'a Request<'r>) -> request::Outcome<Self, Self::Error> {
        let ev = if let Some(ev) = req.headers().get_one("X-Hub-Signature") {
            ev
        } else {
            return Outcome::Failure((Status::BadRequest, "Needs a X-Hub-Signature"));
        };
        Outcome::Success(XHubSignature(ev))
    }
}

#[post("/github-hook", data = "<payload>")]
fn webhook(
    signature: XHubSignature,
    event_header: XGitHubEvent,
    payload: Data,
    ctx: State<handlers::Context>,
) -> Result<(), WebhookError> {
    let event = match event_header.0.parse::<EventName>() {
        Ok(v) => v,
        Err(_) => unreachable!(),
    };

    let mut stream = payload.open().take(1024 * 1024 * 5); // 5 Megabytes
    let mut buf = Vec::new();
    if let Err(err) = stream.read_to_end(&mut buf) {
        log::trace!("failed to read request body: {:?}", err);
        return Err(WebhookError::from(failure::err_msg(
            "failed to read request body",
        )));
    }

    payload::assert_signed(signature.0, &buf).map_err(failure::Error::from)?;
    let payload = String::from_utf8(buf)
        .context("utf-8 payload required")
        .map_err(failure::Error::from)?;
    triagebot::webhook(event, payload, &ctx)
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

    rocket::ignite()
        .manage(gh)
        .manage(ctx)
        .mount("/", routes![webhook])
        .launch();
}
