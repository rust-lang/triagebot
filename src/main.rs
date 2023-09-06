#![allow(clippy::new_without_default)]

use anyhow::Context as _;
use chrono::{Duration, Utc};
use crypto_hash::{hex_digest, Algorithm};
use futures::future::FutureExt;
use futures::StreamExt;
use hyper::{header, Body, Request, Response, Server, StatusCode};
use reqwest::Client;
use route_recognizer::Router;
use std::collections::HashMap;
use std::{env, net::SocketAddr, sync::Arc};
use tokio::{task, time};
use tower::{Service, ServiceExt};
use tracing as log;
use tracing::Instrument;
use triagebot::actions::TEMPLATES;
use triagebot::github::User;
use triagebot::handlers::review_prefs::get_user;
use triagebot::jobs::{jobs, JOB_PROCESSING_CADENCE_IN_SECS, JOB_SCHEDULING_CADENCE_IN_SECS};
use triagebot::ReviewCapacityUser;
use triagebot::{
    db, github,
    handlers::review_prefs::{get_prefs, set_prefs},
    handlers::Context,
    notification_listing, payload, EventName,
};

async fn handle_agenda_request(req: String) -> anyhow::Result<String> {
    if req == "/agenda/lang/triage" {
        return triagebot::agenda::lang().call().await;
    }
    if req == "/agenda/lang/planning" {
        return triagebot::agenda::lang_planning().call().await;
    }

    anyhow::bail!("Unknown agenda; see /agenda for index.")
}

fn validate_data(prefs: &ReviewCapacityUser) -> anyhow::Result<()> {
    if prefs.pto_date_start > prefs.pto_date_end {
        return Err(anyhow::anyhow!(
            "pto_date_start cannot be bigger than pto_date_end"
        ));
    }
    Ok(())
}

async fn exchange_code(code: &str, client_id: &str, client_secret: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let payload =
        serde_json::json!({"client_id":client_id, "client_secret":client_secret, "code":code});

    let tk = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&payload)
        .send()
        .await
        .context("Failed to contact remote host")
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .context("Could not decode response")
        .expect("Error while retrieving the GH token");

    if let Some(err_msg) = tk.get("error_description").cloned() {
        return Err(anyhow::Error::msg(err_msg));
    }
    Ok(tk
        .get("access_token")
        .unwrap()
        .to_string()
        .replace("\"", ""))
}

async fn get_gh_user(access_token: &str) -> anyhow::Result<github::User> {
    let client = Client::new();
    // XXX: weird: the token seems to not be used by the client
    let gh = github::GithubClient::new(client, access_token.to_string());
    gh.get_profile(access_token).await
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
    router.add("/static/:path/:file", "static-assets".to_string());
    let (req, body_stream) = req.into_parts();

    if let Ok(matcher) = router.recognize(req.uri.path()) {
        if matcher.handler().as_str() == "static-assets" {
            let params = matcher.params();
            let path = params.find("path");
            let _file = params.find("file");
            return triagebot::triage::asset(path.unwrap(), _file.unwrap()).await;
        }
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

    if req.uri.path() == "/review-settings" {
        let mut members = vec![];
        // yes, I am an hardcoded admin
        let mut admins = vec!["apiraino".to_string()];
        let gh = github::GithubClient::new_with_default_token(Client::new());

        // check if a session cookie is sent, retrieve the GH username
        let maybe_user_enc = match req.headers.get("Cookie") {
            Some(cookies) => {
                log::debug!("cookies={:?}", cookies);
                let cookie_val = cookies.to_str().unwrap().split(';').filter_map(|_cookie| {
                    let _c = _cookie.split('=').map(|x| x.trim()).collect::<Vec<&str>>();
                    if _c[0] == "triagebot.session".to_string() {
                        Some(_c[1])
                    } else {
                        None
                    }
                });
                let x = cookie_val.last();
                x
            }
            _ => None,
        };

        let db_client = ctx.db.get().await;
        let user = match maybe_user_enc {
            Some(user_enc) => {
                // We have a user in the cookie
                // Verify who this user claims to be
                // format is: {"checksum":"...", "exp":"...", "sub":"...", "uid":"..."}
                let user_check: serde_json::Value = serde_json::from_str(user_enc).unwrap();
                log::debug!("Found claim in cookie: {:?}", user_check);
                let basic_check = create_cookie_content(
                    user_check["sub"].as_str().unwrap(),
                    user_check["uid"].as_i64().unwrap(),
                );
                if basic_check["checksum"] != user_check["checksum"] {
                    return Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Body::empty())
                        .unwrap());
                }

                match get_user(&db_client, user_check["checksum"].as_str().unwrap()).await {
                    Ok(u) => User {
                        login: u.username,
                        id: Some(u.user_id),
                    },
                    Err(err) => {
                        log::debug!("{:?}", err);
                        return Ok(Response::builder()
                            .status(StatusCode::FORBIDDEN)
                            .body(Body::empty())
                            .unwrap());
                    }
                }
            }
            _ => {
                // No username. Did we receive a `code` in the query URL (i.e. did the user went through the GH auth)?
                let client_id = std::env::var("CLIENT_ID").expect("CLIENT_ID is not set");
                let client_secret =
                    std::env::var("CLIENT_SECRET").expect("CLIENT_SECRET is not set");
                if let Some(query) = req.uri.query() {
                    let code =
                        url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == "code");
                    if let Some((_, code)) = code {
                        // generate a token to impersonate the user
                        let maybe_access_token =
                            exchange_code(&code, &client_id, &client_secret).await;
                        if let Err(err_msg) = maybe_access_token {
                            log::debug!("Github auth failed: {}", err_msg);
                            let login_link = format!(
                                "https://github.com/login/oauth/authorize?client_id={}",
                                client_id
                            );
                            return Ok(Response::builder()
                                  .status(StatusCode::OK)
                                  .header(hyper::header::CONTENT_TYPE, "text/html")
                                  .body(Body::from(
                                      format!("<html><body>Fatal error occurred during authentication. Please click <a href='{}'>here</a> to retry.\n\n<br/><br/>If the error persists, please contact an administrator on <a href='https://rust-lang.zulipchat.com/#narrow/stream/242791-t-infra'>Zulip</a>.</body></html>", login_link),
                                  ))
                                  .unwrap());
                        }
                        let access_token = maybe_access_token.unwrap();
                        // Ok, we have an access_token. Retrieve the GH username
                        match get_gh_user(&access_token).await {
                            Ok(user) => user,
                            Err(err) => {
                                log::debug!("Could not retrieve the user from GH: {:?}", err);
                                return Ok(Response::builder()
                                  .status(StatusCode::OK)
                                  .header(hyper::header::CONTENT_TYPE, "text/html")
                                  .body(Body::from(
                                      "<html><body>Fatal error: cannot load the backoffice. Please contact an administrator on <a href='https://rust-lang.zulipchat.com/#narrow/stream/242791-t-infra'>Zulip</a>.\n\n<!-- Hint: cannot retrieve user from the github API --></body></html>",
                                  ))
                                  .unwrap());
                            }
                        }
                    } else {
                        return Ok(Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .body(Body::empty())
                            .unwrap());
                    }
                } else {
                    // no code and no username received: we know nothing about this visitor. Redirect to GH login.
                    return Ok(Response::builder()
                        .status(StatusCode::MOVED_PERMANENTLY)
                        .header(
                            hyper::header::LOCATION,
                            format!(
                                "https://github.com/login/oauth/authorize?client_id={}",
                                client_id
                            ),
                        )
                        .body(Body::empty())
                        .unwrap());
                }
            }
        };

        // Here we have a validated username. From now on, we will trust this user
        let is_admin = admins.contains(&user.login);
        log::debug!("user={}, is admin: {}", user.login, is_admin);

        // get team members from github (raw GH HTTP file retrieval, no auth used)
        // TODO: maybe add some kind of caching for these files
        gh.get_team_members(&mut admins, &mut members, "compiler.toml")
            .await;
        gh.get_team_members(&mut admins, &mut members, "compiler-contributors.toml")
            .await;
        members.sort();
        log::debug!("Members loaded {:?}", members);

        if req.method == hyper::Method::POST {
            let mut c = body_stream;
            let mut payload = Vec::new();
            while let Some(chunk) = c.next().await {
                let chunk = chunk?;
                payload.extend_from_slice(&chunk);
            }
            let prefs = url::form_urlencoded::parse(payload.as_ref())
                .into_owned()
                .collect::<HashMap<String, String>>()
                .into();
            log::debug!("prefs from the web form {:?}", prefs);

            // TODO: maybe add more input validation
            validate_data(&prefs).unwrap();

            // save changes
            let _review_capacity = set_prefs(&db_client, prefs).await.unwrap();
        }

        // Query and return all team member prefs
        let review_capacity = get_prefs(&db_client, &mut members, &user.login, is_admin).await;
        let curr_user_prefs = serde_json::json!(&review_capacity.iter().take(1).last());
        let team_prefs = serde_json::json!(&review_capacity);
        log::debug!("My prefs: {:?}", curr_user_prefs);
        log::debug!("Other team prefs: {:?}", team_prefs);

        let mut context = tera::Context::new();
        context.insert("user_prefs", &curr_user_prefs);
        context.insert("team_prefs", &team_prefs);
        let body = TEMPLATES
            .render("pr-prefs-backoffice.html", &context)
            .unwrap();

        // TODO: compress this response
        let status_code = if req.method == hyper::Method::POST {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        };
        let cookie_exp = Utc::now() + Duration::hours(1);
        let cookie_content = format!(
            "triagebot.session={}; Expires={}; Secure; HttpOnly; SameSite=Strict",
            create_cookie_content(&user.login, user.id.unwrap()).to_string(),
            // RFC 5322: Thu, 31 Dec 2023 23:00:00 GMT
            cookie_exp.format("%a, %d %b %Y %H:%M:%S %Z")
        );
        log::debug!("Cookie content: {}", cookie_content);

        let resp = Response::builder()
            .header(hyper::header::CONTENT_TYPE, "text/html")
            // .header(hyper::header::CONTENT_ENCODING, "gzip")
            .header(
                hyper::header::SET_COOKIE,
                header::HeaderValue::from_str(&cookie_content).unwrap(),
            )
            .status(status_code)
            .body(Body::from(body));

        return Ok(resp.unwrap());
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

/// iss=triagebot, sub=gh username, uid=gh user_id, exp=now+30', checksum=sha256(user data)
fn create_cookie_content(user_login: &str, user_id: i64) -> serde_json::Value {
    let auth_secret = std::env::var("BACKOFFICE_SECRET").expect("BACKOFFICE_SECRET is not set");
    let exp = Utc::now() + Duration::minutes(30);
    let digest = format!("{};{};{}", user_id, user_login, auth_secret);
    let digest = hex_digest(Algorithm::SHA256, &digest.into_bytes());
    serde_json::json!({"iss":"triagebot", "sub":user_login, "uid": user_id, "exp":exp, "checksum":digest})
}

async fn run_server(addr: SocketAddr) -> anyhow::Result<()> {
    let pool = db::ClientPool::new();
    db::run_migrations(&*pool.get().await)
        .await
        .context("database migrations")?;

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

    if !is_scheduled_jobs_disabled() {
        spawn_job_scheduler();
        spawn_job_runner(ctx.clone());
    }

    let agenda = tower::ServiceBuilder::new()
        .buffer(10)
        .layer_fn(|input| {
            tower::util::MapErr::new(
                tower::load_shed::LoadShed::new(tower::limit::RateLimit::new(
                    input,
                    tower::limit::rate::Rate::new(2, std::time::Duration::from_secs(60)),
                )),
                |e| {
                    tracing::error!("agenda request failed: {:?}", e);
                    anyhow::anyhow!("Rate limit of 2 request / 60 seconds exceeded")
                },
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

/// Spawns a background tokio task which runs continuously to queue up jobs
/// to be run by the job runner.
///
/// The scheduler wakes up every `JOB_SCHEDULING_CADENCE_IN_SECS` seconds to
/// check if there are any jobs ready to run. Jobs get inserted into the the
/// database which acts as a queue.
fn spawn_job_scheduler() {
    task::spawn(async move {
        loop {
            let res = task::spawn(async move {
                let pool = db::ClientPool::new();
                let mut interval =
                    time::interval(time::Duration::from_secs(JOB_SCHEDULING_CADENCE_IN_SECS));

                loop {
                    interval.tick().await;
                    db::schedule_jobs(&*pool.get().await, jobs())
                        .await
                        .context("database schedule jobs")
                        .unwrap();
                }
            });

            match res.await {
                Err(err) if err.is_panic() => {
                    /* handle panic in above task, re-launching */
                    tracing::error!("schedule_jobs task died (error={err})");
                    tokio::time::sleep(std::time::Duration::new(5, 0)).await;
                }
                _ => unreachable!(),
            }
        }
    });
}

/// Spawns a background tokio task which runs continuously to run scheduled
/// jobs.
///
/// The runner wakes up every `JOB_PROCESSING_CADENCE_IN_SECS` seconds to
/// check if any jobs have been put into the queue by the scheduler. They
/// will get popped off the queue and run if any are found.
fn spawn_job_runner(ctx: Arc<Context>) {
    task::spawn(async move {
        loop {
            let ctx = ctx.clone();
            let res = task::spawn(async move {
                let pool = db::ClientPool::new();
                let mut interval =
                    time::interval(time::Duration::from_secs(JOB_PROCESSING_CADENCE_IN_SECS));

                loop {
                    interval.tick().await;
                    db::run_scheduled_jobs(&ctx, &*pool.get().await)
                        .await
                        .context("run database scheduled jobs")
                        .unwrap();
                }
            });

            match res.await {
                Err(err) if err.is_panic() => {
                    /* handle panic in above task, re-launching */
                    tracing::error!("run_scheduled_jobs task died (error={err})");
                    tokio::time::sleep(std::time::Duration::new(5, 0)).await;
                }
                _ => unreachable!(),
            }
        }
    });
}

/// Determines whether or not background scheduled jobs should be disabled for
/// the purpose of testing.
///
/// This helps avoid having random jobs run while testing other things.
fn is_scheduled_jobs_disabled() -> bool {
    env::var_os("TRIAGEBOT_TEST_DISABLE_JOBS").is_some()
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
