use reqwest::Client;
use triagebot::github::GithubClient;

mod old_label;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let client = GithubClient::new_with_default_token(Client::new());

    old_label::triage_old_label(
        "rust-lang",
        "rust",
        "E-needs-mcve",
        "triaged", // Exclude e.g. label "AsyncAwait-Triaged"
        chrono::Duration::days(30 * 12 * 4),
        &client,
    )
    .await?;

    Ok(())
}
