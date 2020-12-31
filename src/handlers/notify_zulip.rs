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
    Closed,
    Reopened,
}

pub(super) fn parse_input(
    _ctx: &Context,
    event: &IssuesEvent,
    config: Option<&NotifyZulipConfig>,
) -> Result<Option<NotifyZulipInput>, String> {
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

        match event.action {
            IssuesAction::Labeled if config.message_on_add.is_some() => {
                Ok(Some(NotifyZulipInput {
                    notification_type: NotificationType::Labeled,
                }))
            }
            IssuesAction::Unlabeled if config.message_on_remove.is_some() => {
                Ok(Some(NotifyZulipInput {
                    notification_type: NotificationType::Unlabeled,
                }))
            }
            IssuesAction::Closed if config.message_on_close.is_some() => {
                Ok(Some(NotifyZulipInput {
                    notification_type: NotificationType::Closed,
                }))
            }
            IssuesAction::Reopened if config.message_on_reopen.is_some() => {
                Ok(Some(NotifyZulipInput {
                    notification_type: NotificationType::Reopened,
                }))
            }
            _ => Ok(None),
        }
    } else {
        Ok(None)
    }
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
    // Truncate to 60 chars (a Zulip limitation)
    let mut chars = topic.char_indices().skip(59);
    if let (Some((len, _)), Some(_)) = (chars.next(), chars.next()) {
        topic.truncate(len);
        topic.push('…');
    }

    let mut msg = match input.notification_type {
        NotificationType::Labeled => config.message_on_add.as_ref().unwrap().clone(),
        NotificationType::Unlabeled => config.message_on_remove.as_ref().unwrap().clone(),
        NotificationType::Closed => config.message_on_close.as_ref().unwrap().clone(),
        NotificationType::Reopened => config.message_on_reopen.as_ref().unwrap().clone(),
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
