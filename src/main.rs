#![allow(clippy::new_without_default)]

use futures::{
    compat::{Future01CompatExt, Stream01CompatExt},
    future::{FutureExt, TryFutureExt},
    stream::StreamExt,
};
use hyper::{header, service::service_fn, Body, Request, Response, Server, StatusCode};
use reqwest::r#async::Client;
use std::{env, net::SocketAddr, sync::Arc};
use triagebot::{github, handlers::Context, payload, EventName};
use uuid::Uuid;

mod logger;

async fn serve_req(req: Request<Body>, ctx: Arc<Context>) -> Result<Response<Body>, hyper::Error> {
    log::info!("request = {:?}", req);
    let (req, body_stream) = req.into_parts();
    if req.uri.path() != "/github-hook" {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap());
    }
    if req.method != hyper::Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "POST")
            .body(Body::empty())
            .unwrap());
    }
    let event = if let Some(ev) = req.headers.get("X-GitHub-Event") {
        let ev = match ev.to_str().ok() {
            Some(v) => v,
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("X-GitHub-Event header must be UTF-8 encoded"))
                    .unwrap());
            }
        };
        match ev.parse::<EventName>() {
            Ok(v) => v,
            Err(_) => unreachable!(),
        }
    } else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("X-GitHub-Event header must be set"))
            .unwrap());
    };
    log::debug!("event={}", event);
    let signature = if let Some(sig) = req.headers.get("X-Hub-Signature") {
        match sig.to_str().ok() {
            Some(v) => v,
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("X-Hub-Signature header must be UTF-8 encoded"))
                    .unwrap());
            }
        }
    } else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("X-Hub-Signature header must be set"))
            .unwrap());
    };
    log::debug!("signature={}", signature);

    let mut c = body_stream.compat();
    let mut payload = Vec::new();
    while let Some(chunk) = c.next().await {
        let chunk = chunk?;
        payload.extend_from_slice(&chunk);
    }

    if let Err(_) = payload::assert_signed(signature, &payload) {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from("Wrong signature"))
            .unwrap());
    }
    let payload = match String::from_utf8(payload) {
        Ok(p) => p,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("Payload must be UTF-8"))
                .unwrap());
        }
    };

    match triagebot::webhook(event, payload, &ctx).await {
        Ok(()) => {}
        Err(err) => {
            log::error!("request failed: {:?}", err);
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("request failed"))
                .unwrap());
        }
    }

    Ok(Response::new(Body::from("processed request")))
}

async fn run_server(addr: SocketAddr) {
    log::info!("Listening on http://{}", addr);

    let client = Client::new();
    let gh = github::GithubClient::new(
        client.clone(),
        env::var("GITHUB_API_TOKEN").expect("Missing GITHUB_API_TOKEN"),
    );
    let ctx = Arc::new(Context {
        username: github::User::current(&gh).await.unwrap().login,
        github: gh,
    });

    let serve_future = Server::bind(&addr).serve(move || {
        let ctx = ctx.clone();
        service_fn(move |req| {
            let ctx = ctx.clone();
            let uuid = Uuid::new_v4();
            logger::LogFuture::new(
                uuid,
                serve_req(req, ctx).map(move |mut resp| {
                    if let Ok(resp) = &mut resp {
                        resp.headers_mut()
                            .insert("X-Request-Id", uuid.to_string().parse().unwrap());
                    }
                    log::info!("response = {:?}", resp);
                    resp
                }),
            )
            .boxed()
            .compat()
        })
    });

    if let Err(e) = serve_future.compat().await {
        eprintln!("server error: {}", e);
    }
}

fn main() {
    dotenv::dotenv().ok();
    logger::init();

    let port = env::var("PORT")
        .ok()
        .map(|p| p.parse::<u16>().expect("parsed PORT"))
        .unwrap_or(8000);
    let addr = ([0, 0, 0, 0], port).into();
    hyper::rt::run(run_server(addr).unit_error().boxed().compat());
}
