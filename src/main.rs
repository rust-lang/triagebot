#![allow(clippy::new_without_default)]

use anyhow::Context as _;
use futures::{future::FutureExt, stream::StreamExt};
use hyper::{header, Body, Request, Response, Server, StatusCode};
use reqwest::Client;
use std::{env, net::SocketAddr, sync::Arc};
use triagebot::{db, github, handlers::Context, notification_listing, payload, EventName};
use uuid::Uuid;

mod logger;

async fn serve_req(req: Request<Body>, ctx: Arc<Context>) -> Result<Response<Body>, hyper::Error> {
    log::info!("request = {:?}", req);
    let (req, body_stream) = req.into_parts();
    if req.uri.path() == "/" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("Triagebot is awaiting triage."))
            .unwrap());
    }
    if req.uri.path() == "/notifications" {
        if let Some(query) = req.uri.query() {
            let user = url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == "user");
            if let Some((_, name)) = user {
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from(
                        notification_listing::render(&ctx.db, &*name).await,
                    ))
                    .unwrap());
            }
        }

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(String::from(
                "Please provide `?user=<username>` query param on URL.",
            )))
            .unwrap());
    }
    if req.uri.path() == "/zulip-hook" {
        let mut c = body_stream;
        let mut payload = Vec::new();
        while let Some(chunk) = c.next().await {
            let chunk = chunk?;
            payload.extend_from_slice(&chunk);
        }

        let req = match serde_json::from_slice(&payload) {
            Ok(r) => r,
            Err(e) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from(format!(
                        "Did not send valid JSON request: {}",
                        e
                    )))
                    .unwrap());
            }
        };

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(triagebot::zulip::respond(&ctx, req).await))
            .unwrap());
    }
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

    let mut c = body_stream;
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
        Ok(true) => Ok(Response::new(Body::from("processed request"))),
        Ok(false) => Ok(Response::new(Body::from("ignored request"))),
        Err(err) => {
            log::error!("request failed: {:?}", err);
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!("request failed: {:?}", err)))
                .unwrap())
        }
    }
}

async fn run_server(addr: SocketAddr) -> anyhow::Result<()> {
    log::info!("Listening on http://{}", addr);

    let client = Client::new();
    let db_client = Context::make_db_client(&client)
        .await
        .context("open database connection")?;
    db::run_migrations(&db_client)
        .await
        .context("database migrations")?;

    let gh = github::GithubClient::new(
        client.clone(),
        env::var("GITHUB_API_TOKEN").expect("Missing GITHUB_API_TOKEN"),
    );
    let ctx = Arc::new(Context {
        username: github::User::current(&gh).await.unwrap().login,
        db: db_client,
        github: gh,
    });

    let svc = hyper::service::make_service_fn(move |_conn| {
        let ctx = ctx.clone();
        async move {
            let uuid = Uuid::new_v4();
            Ok::<_, hyper::Error>(hyper::service::service_fn(move |req| {
                logger::LogFuture::new(
                    uuid,
                    serve_req(req, ctx.clone()).map(move |mut resp| {
                        if let Ok(resp) = &mut resp {
                            resp.headers_mut()
                                .insert("X-Request-Id", uuid.to_string().parse().unwrap());
                        }
                        log::info!("response = {:?}", resp);
                        resp
                    }),
                )
            }))
        }
    });
    let serve_future = Server::bind(&addr).serve(svc);

    serve_future.await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    logger::init();

    let port = env::var("PORT")
        .ok()
        .map(|p| p.parse::<u16>().expect("parsed PORT"))
        .unwrap_or(8000);
    let addr = ([0, 0, 0, 0], port).into();
    if let Err(e) = run_server(addr).await {
        eprintln!("Failed to run server: {}", e);
    }
}
