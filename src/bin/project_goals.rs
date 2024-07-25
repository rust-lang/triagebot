use triagebot::{github::GithubClient, handlers::project_goals};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let mut dry_run = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            _ => {
                eprintln!("Usage: project_goals [--dry-run]");
                std::process::exit(1);
            }
        }
    }

    let gh = GithubClient::new_from_env();
    project_goals::ping_project_goals_owners(&gh, dry_run).await?;

    Ok(())
}
