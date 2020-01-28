#![allow(clippy::new_without_default)]

use anyhow::Context as _;
use futures::{future::FutureExt, stream::StreamExt};
use hyper::{header, Body, Request, Response, Server, StatusCode};
use native_tls::{Certificate, TlsConnector};
use postgres_native_tls::MakeTlsConnector;
use reqwest::Client;
use std::{env, net::SocketAddr, sync::Arc};
use triagebot::{github, handlers::Context, payload, EventName};
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

const CERT_URL: &str = "https://s3.amazonaws.com/rds-downloads/rds-ca-2019-root.pem";

async fn connect_to_db(client: Client) -> anyhow::Result<tokio_postgres::Client> {
    let db_url = env::var("DATABASE_URL").expect("needs DATABASE_URL");
    if db_url.contains("rds.amazonaws.com") {
        let resp = client
            .get(CERT_URL)
            .send()
            .await
            .context("failed to get RDS cert")?;
        let cert = resp.bytes().await.context("faield to get RDS cert body")?;
        let cert = Certificate::from_pem(&cert).context("made certificate")?;
        let connector = TlsConnector::builder()
            .add_root_certificate(cert)
            .build()
            .context("built TlsConnector")?;
        let connector = MakeTlsConnector::new(connector);

        let (db_client, connection) = match tokio_postgres::connect(&db_url, connector).await {
            Ok(v) => v,
            Err(e) => {
                anyhow::bail!("failed to connect to DB: {}", e);
            }
        };
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("database connection error: {}", e);
            }
        });

        Ok(db_client)
    } else {
        eprintln!("Warning: Non-TLS connection to non-RDS DB");
        let (db_client, connection) =
            match tokio_postgres::connect(&db_url, tokio_postgres::NoTls).await {
                Ok(v) => v,
                Err(e) => {
                    anyhow::bail!("failed to connect to DB: {}", e);
                }
            };
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("database connection error: {}", e);
            }
        });

        Ok(db_client)
    }
}

async fn run_server(addr: SocketAddr) {
    log::info!("Listening on http://{}", addr);

    let client = Client::new();
    let db_client = match connect_to_db(client.clone()).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to connect to db: {}", e);
            return;
        }
    };

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

    if let Err(e) = serve_future.await {
        eprintln!("server error: {}", e);
    }
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
    run_server(addr).await;
}
