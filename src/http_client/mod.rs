use chrono::{Datelike, Duration, NaiveDate};
use hyper::StatusCode;

pub async fn get_compiler_perf_triage_logs(
    gh_client: &crate::github::GithubClient,
    date: NaiveDate,
) -> anyhow::Result<String> {
    // Perf triage logs are usually merged a few days before building the T-compiler triage meeting
    let try_days = [
        date.to_string(),
        (date - Duration::days(1)).to_string(),
        (date - Duration::days(2)).to_string(),
        (date - Duration::days(3)).to_string(),
        (date - Duration::days(4)).to_string(),
    ];

    let repo = crate::github::IssueRepository {
        organization: "rust-lang".to_string(),
        repository: "rustc-perf".to_string(),
    };
    let resp = gh_client
        .get_contents(&repo, format!("/triage/{}", date.year()))
        .await?;

    let mut files = resp
        .iter()
        .map(|e| (&e.name, &e.download_url))
        .collect::<Vec<(&String, &String)>>();
    files.sort_by_key(|k| k.0);
    files.reverse();

    // iterate a few days back and look for a file matching that day
    // if yes, download that file
    for day in try_days {
        for f in &files {
            if f.0.starts_with(&day) {
                let resp_file = reqwest::get(f.1).await?;
                if resp_file.status() == StatusCode::OK {
                    return Ok(resp_file
                        .text()
                        .await
                        .unwrap()
                        .replace("❌", "")
                        .replace("✅", "")
                        .replace(" <br /> ", ""));
                }
            }
        }
    }
    anyhow::bail!("")
}
