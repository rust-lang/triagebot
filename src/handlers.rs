use crate::config::{self, ConfigurationError};
use crate::github::{Event, GithubClient};
use anyhow::Context as _;
use futures::future::BoxFuture;
use native_tls::{Certificate, TlsConnector};
use postgres_native_tls::MakeTlsConnector;
use std::fmt;
use tokio_postgres::Client as DbClient;

#[derive(Debug)]
pub enum HandlerError {
    Message(String),
    Other(anyhow::Error),
}

impl std::error::Error for HandlerError {}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HandlerError::Message(msg) => write!(f, "{}", msg),
            HandlerError::Other(_) => write!(f, "An internal error occurred."),
        }
    }
}

macro_rules! handlers {
    ($($name:ident = $handler:expr,)*) => {
        $(mod $name;)*
        mod notification;

        pub async fn handle(ctx: &Context, event: &Event) -> Result<(), HandlerError> {
            $(
            if let Some(input) = Handler::parse_input(
                    &$handler, ctx, event).map_err(HandlerError::Message)? {
                let config = match config::get(&ctx.github, event.repo_name()).await {
                    Ok(config) => config,
                    Err(e @ ConfigurationError::Missing) => {
                        return Err(HandlerError::Message(e.to_string()));
                    }
                    Err(e @ ConfigurationError::Toml(_)) => {
                        return Err(HandlerError::Message(e.to_string()));
                    }
                    Err(e @ ConfigurationError::Http(_)) => {
                        return Err(HandlerError::Other(e.into()));
                    }
                };
                if let Some(config) = &config.$name {
                    Handler::handle_input(&$handler, ctx, config, event, input).await.map_err(HandlerError::Other)?;
                } else {
                    return Err(HandlerError::Message(format!(
                        "The feature `{}` is not enabled in this repository.\n\
                         To enable it add its section in the `triagebot.toml` \
                         in the root of the repository.",
                        stringify!($name)
                    )));
                }
            })*

            if let Err(e) = notification::handle(ctx, event).await {
                log::error!("failed to process event {:?} with notification handler: {:?}", event, e);
            }

            Ok(())
        }
    }
}

handlers! {
    assign = assign::AssignmentHandler,
    relabel = relabel::RelabelHandler,
    ping = ping::PingHandler,
    nominate = nominate::NominateHandler,
    prioritize = prioritize::PrioritizeHandler,
    //tracking_issue = tracking_issue::TrackingIssueHandler,
}

pub struct Context {
    pub github: GithubClient,
    pub db: DbClient,
    pub username: String,
}

const CERT_URL: &str = "https://s3.amazonaws.com/rds-downloads/rds-ca-2019-root.pem";

impl Context {
    pub async fn make_db_client(
        client: &reqwest::Client,
    ) -> anyhow::Result<tokio_postgres::Client> {
        let db_url = std::env::var("DATABASE_URL").expect("needs DATABASE_URL");
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
}

pub trait Handler: Sync + Send {
    type Input;
    type Config;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, String>;

    fn handle_input<'a>(
        &self,
        ctx: &'a Context,
        config: &'a Self::Config,
        event: &'a Event,
        input: Self::Input,
    ) -> BoxFuture<'a, anyhow::Result<()>>;
}
