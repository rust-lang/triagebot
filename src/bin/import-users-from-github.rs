use reqwest::Client;
use tracing::{debug, info};
use triagebot::db::notifications::record_username;
use triagebot::github::User;
use triagebot::handlers::review_prefs::{add_prefs, delete_prefs, get_prefs};
use triagebot::{db::make_client, github};

// Import and synchronization:
// 1. Download teams and retrieve those listed in $NEW_PR_ASSIGNMENT_TEAMS
// 2. Add missing team members to the review preferences table
// 3. Delete preferences for members not present in the team roster anymore

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let gh = github::GithubClient::new_with_default_token(Client::new());
    let db_client = make_client().await.unwrap();
    let teams_data = triagebot::team_data::teams(&gh).await?;

    // 1. get team members
    let x = std::env::var("NEW_PR_ASSIGNMENT_TEAMS")
        .expect("NEW_PR_ASSIGNMENT_TEAMS env var must be set");
    let allowed_teams = x.split(",").collect::<Vec<&str>>();
    info!("Will download members for teams: {:?}", allowed_teams);
    for team in &allowed_teams {
        let members = teams_data.teams.get(*team).unwrap();
        let team_members = members
            .members
            .iter()
            .map(|tm| User {
                login: tm.github.clone(),
                id: Some(tm.github_id as i64),
            })
            .collect::<Vec<_>>();
        debug!("Team {} members loaded: {:?}", team, team_members);

        // get team members review capacity
        let team_review_prefs = get_prefs(
            &db_client,
            &team_members
                .iter()
                .map(|tm| tm.login.clone())
                .collect::<Vec<String>>(),
            "apiraino",
            true,
        )
        .await;

        // 2. Add missing team members to the review preferences table
        for member in &team_members {
            if !team_review_prefs
                .iter()
                .find(|rec| rec.username == member.login)
                .is_some()
            {
                debug!(
                    "Team member {:?} was NOT found in the prefs DB table",
                    member
                );

                // ensure this person exists in the users DB table first
                let team_member = team_members
                    .iter()
                    .find(|m| m.login == member.login)
                    .expect(&format!(
                        "Could not find member {:?} in team {}",
                        member, team
                    ));
                let _ = record_username(&db_client, team_member.id.unwrap() as i64, &member.login)
                    .await;

                // Create a record in the review_capacity DB table for this member with some defaults
                let _ = add_prefs(&db_client, team_member.id.unwrap() as i64).await?;
                info!("Added team member {}", &team_member.login);
            }
        }

        // 3. delete prefs for members not present in the team roster anymore
        let removed_members = team_review_prefs
            .iter()
            .filter(|tm| {
                !team_members.contains(&User {
                    id: Some(tm.user_id),
                    login: tm.username.clone(),
                })
            })
            .map(|tm| tm.user_id)
            .collect::<Vec<i64>>();
        if !removed_members.is_empty() {
            let _ = delete_prefs(&db_client, &removed_members).await?;
            info!("Delete preferences for team members {:?}", &removed_members);
        }
        info!("Finished updating review prefs for team {}", team);
    }

    info!("Import/Sync job finished");
    Ok(())
}
