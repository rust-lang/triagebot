#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRepository {
    pub organization: String,
    pub repository: String,
}

impl fmt::Display for IssueRepository {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.organization, self.repository)
    }
}

impl IssueRepository {
    fn url(&self, client: &GithubClient) -> String {
        format!(
            "{}/repos/{}/{}",
            client.api_url, self.organization, self.repository
        )
    }

    pub(crate) fn full_repo_name(&self) -> String {
        format!("{}/{}", self.organization, self.repository)
    }

    async fn has_label(&self, client: &GithubClient, label: &str) -> anyhow::Result<bool> {
        #[allow(clippy::redundant_pattern_matching)]
        let url = format!("{}/labels/{}", self.url(client), label);
        match client.send_req(client.get(&url)).await {
            Ok(_) => Ok(true),
            Err(e) => {
                if e.downcast_ref::<reqwest::Error>()
                    .is_some_and(|e| e.status() == Some(StatusCode::NOT_FOUND))
                {
                    Ok(false)
                } else {
                    Err(e)
                }
            }
        }
    }
}

// Collaborator permission

#[derive(Debug, serde::Deserialize)]
pub struct CollaboratorPermission {
    pub permission: String,
}

impl IssueRepository {
    pub(crate) async fn collaborator_permission(
        &self,
        client: &GithubClient,
        username: &str,
    ) -> anyhow::Result<CollaboratorPermission> {
        let url = format!("{}/collaborators/{username}/permission", self.url(client));
        let permission = client.json(client.get(&url)).await?;
        Ok(permission)
    }
}
