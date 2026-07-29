use clap::Parser;
use triagebot::team_data::TeamClient;
use triagebot::zulip::client::ZulipClient;
use triagebot::{github::GithubClient, handlers::project_goals};

/// A basic example
#[derive(Parser, Debug)]
struct Opt {
    /// If specified, no messages are sent.
    #[arg(long)]
    dry_run: bool,

    /// Goals updated within this threshold (in days) will not be pinged.
    days_threshold: i64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let opt = Opt::parse();
    let gh = GithubClient::new_from_env();
    let zulip = ZulipClient::new_from_env();
    let team_api = TeamClient::new_from_env();
    project_goals::ping_project_goals_owners(
        &gh,
        &zulip,
        &team_api,
        opt.dry_run,
        opt.days_threshold,
    )
    .await?;

    Ok(())
}
