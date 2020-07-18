use crate::{
    config::PrioritizeConfig,
    github::{self, Event},
    handlers::Context,
};
use parser::command::prioritize::PrioritizeCommand;

pub(super) async fn handle_command(
    ctx: &Context,
    config: &PrioritizeConfig,
    event: &Event,
    _: PrioritizeCommand,
) -> anyhow::Result<()> {
    let issue = event.issue().unwrap();
    let mut labels = issue.labels().to_owned();

    // Don't add the label if it's already there
    if !labels.iter().any(|l| l.name == config.label) {
        labels.push(github::Label {
            name: config.label.to_owned(),
        });
    }

    issue.set_labels(&ctx.github, labels).await?;
    Ok(())
}
