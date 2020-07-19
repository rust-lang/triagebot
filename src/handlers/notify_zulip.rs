use crate::{
    config::NotifyZulipConfig,
    github::{IssuesAction, IssuesEvent},
    handlers::Context,
};

pub(super) struct NotifyZulipInput {
    notification_type: NotificationType,
}

pub(super) enum NotificationType {
    Labeled,
    Unlabeled,
}

pub(super) fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&NotifyZulipConfig>,
) -> Result<Option<NotifyZulipInput>, String> {
    if let IssuesAction::Labeled | IssuesAction::Unlabeled = event.action {
        let applied_label = &event.label.as_ref().expect("label").name;
        if let Some(config) = config.and_then(|c| c.labels.get(applied_label)) {
            for label in &config.required_labels {
                let pattern = match glob::Pattern::new(label) {
                    Ok(pattern) => pattern,
                    Err(err) => {
                        log::error!("Invalid glob pattern: {}", err);
                        continue;
                    }
                };
                if !event
                    .issue
                    .labels()
                    .iter()
                    .any(|l| pattern.matches(&l.name))
                {
                    // Issue misses a required label, ignore this event
                    return Ok(None);
                }
            }

            if event.action == IssuesAction::Labeled && config.message_on_add.is_some() {
                return Ok(Some(NotifyZulipInput {
                    notification_type: NotificationType::Labeled,
                }));
            } else if config.message_on_remove.is_some() {
                return Ok(Some(NotifyZulipInput {
                    notification_type: NotificationType::Unlabeled,
                }));
            }
        }
    }
    Ok(None)
}

pub(super) async fn handle_input<'a>(
    ctx: &Context,
    config: &NotifyZulipConfig,
    event: &IssuesEvent,
    input: NotifyZulipInput,
) -> anyhow::Result<()> {
    let config = config
        .labels
        .get(&event.label.as_ref().unwrap().name)
        .unwrap();

    let mut topic = config.topic.clone();
    topic = topic.replace("{number}", &event.issue.number.to_string());
    topic = topic.replace("{title}", &event.issue.title);
    topic.truncate(60); // Zulip limitation

    let mut msg = match input.notification_type {
        NotificationType::Labeled => config.message_on_add.as_ref().unwrap().clone(),
        NotificationType::Unlabeled => config.message_on_remove.as_ref().unwrap().clone(),
    };

    msg = msg.replace("{number}", &event.issue.number.to_string());
    msg = msg.replace("{title}", &event.issue.title);

    let zulip_req = crate::zulip::MessageApiRequest {
        recipient: crate::zulip::Recipient::Stream {
            id: config.zulip_stream,
            topic: &topic,
        },
        content: &msg,
    };
    zulip_req.send(&ctx.github.raw()).await?;

    Ok(())
}
