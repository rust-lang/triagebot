use crate::github::GithubClient;
use crate::registry::HandleRegistry;
use std::sync::Arc;

//mod assign;
mod label;
//mod tracking_issue;

pub fn register_all(registry: &mut HandleRegistry, client: GithubClient, username: Arc<String>) {
    registry.register(label::LabelHandler {
        client: client.clone(),
        username: username.clone(),
    });
    //registry.register(assign::AssignmentHandler {
    //    client: client.clone(),
    //});
    //registry.register(tracking_issue::TrackingIssueHandler {
    //    client: client.clone(),
    //});
}
