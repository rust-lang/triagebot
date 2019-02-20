use crate::github::GithubClient;
use crate::registry::HandleRegistry;

//mod assign;
mod label;
//mod tracking_issue;

pub fn register_all(registry: &mut HandleRegistry, client: GithubClient) {
    registry.register(label::LabelHandler {
        client: client.clone(),
    });
    //registry.register(assign::AssignmentHandler {
    //    client: client.clone(),
    //});
    //registry.register(tracking_issue::TrackingIssueHandler {
    //    client: client.clone(),
    //});
}
