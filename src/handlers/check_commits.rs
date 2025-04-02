use anyhow::bail;

use super::Context;
use crate::{
    config::Config,
    github::{Event, IssuesAction},
};

mod modified_submodule;
mod non_default_branch;

pub(super) async fn handle(ctx: &Context, event: &Event, config: &Config) -> anyhow::Result<()> {
    let Event::Issue(event) = event else {
        return Ok(());
    };

    if !matches!(event.action, IssuesAction::Opened) || !event.issue.is_pr() {
        return Ok(());
    }

    let Some(diff) = event.issue.diff(&ctx.github).await? else {
        bail!(
            "expected issue {} to be a PR, but the diff could not be determined",
            event.issue.number
        )
    };

    let mut warnings = Vec::new();

    if let Some(assign_config) = &config.assign {
        // For legacy reasons the non-default-branch and modifies-submodule warnings
        // are behind the `[assign]` config.

        if let Some(exceptions) = assign_config
            .warn_non_default_branch
            .enabled_and_exceptions()
        {
            warnings.extend(non_default_branch::non_default_branch(exceptions, event));
        }
        warnings.extend(modified_submodule::modifies_submodule(diff));
    }

    if !warnings.is_empty() {
        let warnings: Vec<_> = warnings
            .iter()
            .map(|warning| format!("* {warning}"))
            .collect();
        let warning = format!(":warning: **Warning** :warning:\n\n{}", warnings.join("\n"));
        event.issue.post_comment(&ctx.github, &warning).await?;
    };

    Ok(())
}
