#![allow(clippy::new_without_default)]

use anyhow::Context as _;
use futures::future::FutureExt;
use futures::StreamExt;
use hyper::{header, Body, Request, Response, Server, StatusCode};
use reqwest::Client;
use route_recognizer::Router;
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tower::{Service, ServiceExt};
use tracing as log;
use tracing::Instrument;
use triagebot::{db, github, handlers::Context, notification_listing, payload, EventName};
use tokio::{task, time::sleep};

const JOB_PROCESSING_CADENCE_IN_SECS: u64 = 60;

async fn handle_agenda_request(req: String) -> anyhow::Result<String> {
    if req == "/agenda/lang/triage" {
        return triagebot::agenda::lang().call().await;
    }
    if req == "/agenda/lang/planning" {
        return triagebot::agenda::lang_planning().call().await;
    }

    anyhow::bail!("Unknown agenda; see /agenda for index.")
}

async fn serve_req(
    req: Request<Body>,
    ctx: Arc<Context>,
    mut agenda: impl Service<String, Response = String, Error = tower::BoxError>,
) -> Result<Response<Body>, hyper::Error> {
    log::info!("request = {:?}", req);
    let mut router = Router::new();
    router.add("/triage", "index".to_string());
    router.add("/triage/:owner/:repo", "pulls".to_string());
    let (req, body_stream) = req.into_parts();

    if let Ok(matcher) = router.recognize(req.uri.path()) {
        if matcher.handler().as_str() == "pulls" {
            let params = matcher.params();
            let owner = params.find("owner");
            let repo = params.find("repo");
            return triagebot::triage::pulls(ctx, owner.unwrap(), repo.unwrap()).await;
        } else {
            return triagebot::triage::index();
        }
    }

    if req.uri.path() == "/agenda" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(triagebot::agenda::INDEX))
            .unwrap());
    }
    if req.uri.path() == "/agenda/lang/triage" || req.uri.path() == "/agenda/lang/planning" {
        match agenda
            .ready()
            .await
            .expect("agenda keeps running")
            .call(req.uri.path().to_owned())
            .await
        {
            Ok(agenda) => {
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from(agenda))
                    .unwrap())
            }
            Err(err) => {
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from(err.to_string()))
                    .unwrap())
            }
        }
    }

    if req.uri.path() == "/" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("Triagebot is awaiting triage."))
            .unwrap());
    }
    if req.uri.path() == "/bors-commit-list" {
        let res = db::rustc_commits::get_commits_with_artifacts(&*ctx.db.get().await).await;
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from(format!("{:?}", e)))
                    .unwrap());
            }
        };
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&res).unwrap()))
            .unwrap());
    }
    if req.uri.path() == "/notifications" {
        if let Some(query) = req.uri.query() {
            let user = url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == "user");
            if let Some((_, name)) = user {
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from(
                        notification_listing::render(&ctx.db.get().await, &*name).await,
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
    let pool = db::ClientPool::new();
    db::run_migrations(&*pool.get().await)
        .await
        .context("database migrations")?;  

    // spawning a background task that will run the scheduled jobs
    // every JOB_PROCESSING_CADENCE_IN_SECS
    task::spawn(async move {
        let pool = db::ClientPool::new();

        loop { 
            db::run_scheduled_jobs(&*pool.get().await)
                .await
                .context("run database scheduled jobs").unwrap(); 

            sleep(Duration::from_secs(JOB_PROCESSING_CADENCE_IN_SECS)).await;
        }
    });

    let client = Client::new();
    let gh = github::GithubClient::new_with_default_token(client.clone());
    let oc = octocrab::OctocrabBuilder::new()
        .personal_token(github::default_token_from_env())
        .build()
        .expect("Failed to build octograb.");
    let ctx = Arc::new(Context {
        username: String::from("rustbot"),
        db: pool,
        github: gh,
        octocrab: oc,
    });

    let agenda = tower::ServiceBuilder::new()
        .buffer(10)
        .layer_fn(|input| {
            tower::util::MapErr::new(
                tower::load_shed::LoadShed::new(tower::limit::RateLimit::new(
                    input,
                    tower::limit::rate::Rate::new(2, std::time::Duration::from_secs(60)),
                )),
                |_| anyhow::anyhow!("Rate limit of 2 request / 60 seconds exceeded"),
            )
        })
        .service_fn(handle_agenda_request);

    let svc = hyper::service::make_service_fn(move |_conn| {
        let ctx = ctx.clone();
        let agenda = agenda.clone();
        async move {
            Ok::<_, hyper::Error>(hyper::service::service_fn(move |req| {
                let uuid = uuid::Uuid::new_v4();
                let span = tracing::span!(tracing::Level::INFO, "request", ?uuid);
                serve_req(req, ctx.clone(), agenda.clone())
                    .map(move |mut resp| {
                        if let Ok(resp) = &mut resp {
                            resp.headers_mut()
                                .insert("X-Request-Id", uuid.to_string().parse().unwrap());
                        }
                        log::info!("response = {:?}", resp);
                        resp
                    })
                    .instrument(span)
            }))
        }
    });
    log::info!("Listening on http://{}", addr);

    let serve_future = Server::bind(&addr).serve(svc);

    serve_future.await?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::Subscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(std::env::var_os("DISABLE_COLOR").is_none())
        .try_init()
        .unwrap();

    let port = env::var("PORT")
        .ok()
        .map(|p| p.parse::<u16>().expect("parsed PORT"))
        .unwrap_or(8000);
    let addr = ([0, 0, 0, 0], port).into();
    if let Err(e) = run_server(addr).await {
        eprintln!("Failed to run server: {:?}", e);
    }
}
