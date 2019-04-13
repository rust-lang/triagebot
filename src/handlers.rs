use crate::github::GithubClient;
use crate::registry::HandleRegistry;

//mod assign;
mod label;
//mod tracking_issue;

pub struct Context {
    pub github: GithubClient,
    pub username: String,
}

pub fn register_all(registry: &mut HandleRegistry) {
    registry.register(label::LabelHandler);
    //registry.register(assign::AssignmentHandler {
    //    client: client.clone(),
    //});
    //registry.register(tracking_issue::TrackingIssueHandler {
    //    client: client.clone(),
    //});
}
