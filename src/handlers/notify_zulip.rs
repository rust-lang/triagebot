use crate::{
    config::{NotifyZulipConfig, NotifyZulipLabelConfig},
    github::{Issue, IssuesAction, IssuesEvent, Label},
    handlers::Context,
};

pub(super) struct NotifyZulipInput {
    notification_type: NotificationType,
    /// Label that triggered this notification.
    ///
    /// For example, if an `I-prioritize` issue is closed,
    /// this field will be `I-prioritize`.
    label: Label,
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
) -> Result<Option<Vec<NotifyZulipInput>>, String> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    match event.action {
        IssuesAction::Labeled | IssuesAction::Unlabeled => {
            let applied_label = event.label.as_ref().expect("label").clone();
            Ok(config
                .labels
                .get(&applied_label.name)
                .and_then(|label_config| {
                    parse_label_change_input(event, applied_label, label_config)
                })
                .map(|input| vec![input]))
        }
        IssuesAction::Closed | IssuesAction::Reopened => {
            Ok(Some(parse_close_reopen_input(event, config)))
        }
        _ => Ok(None),
    }
}

fn parse_label_change_input(
    event: &IssuesEvent,
    label: Label,
    config: &NotifyZulipLabelConfig,
) -> Option<NotifyZulipInput> {
    if !has_all_required_labels(&event.issue, config) {
        // Issue misses a required label, ignore this event
        return None;
    }

    match event.action {
        IssuesAction::Labeled if config.message_on_add.is_some() => Some(NotifyZulipInput {
            notification_type: NotificationType::Labeled,
            label,
        }),
        IssuesAction::Unlabeled if config.message_on_remove.is_some() => Some(NotifyZulipInput {
            notification_type: NotificationType::Unlabeled,
            label,
        }),
        _ => None,
    }
}

fn parse_close_reopen_input(
    event: &IssuesEvent,
    global_config: &NotifyZulipConfig,
) -> Vec<NotifyZulipInput> {
    event
        .issue
        .labels
        .iter()
        .cloned()
        .filter_map(|label| {
            global_config
                .labels
                .get(&label.name)
                .map(|config| (label, config))
        })
        .flat_map(|(label, config)| {
            if !has_all_required_labels(&event.issue, config) {
                // Issue misses a required label, ignore this event
                return None;
            }

            match event.action {
                IssuesAction::Closed if config.message_on_close.is_some() => {
                    Some(NotifyZulipInput {
                        notification_type: NotificationType::Closed,
                        label,
                    })
                }
                IssuesAction::Reopened if config.message_on_reopen.is_some() => {
                    Some(NotifyZulipInput {
                        notification_type: NotificationType::Reopened,
                        label,
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn has_all_required_labels(issue: &Issue, config: &NotifyZulipLabelConfig) -> bool {
    for req_label in &config.required_labels {
        let pattern = match glob::Pattern::new(req_label) {
            Ok(pattern) => pattern,
            Err(err) => {
                log::error!("Invalid glob pattern: {}", err);
                continue;
            }
        };
        if !issue.labels().iter().any(|l| pattern.matches(&l.name)) {
            return false;
        }
    }

    true
}

pub(super) async fn handle_input<'a>(
    ctx: &Context,
    config: &NotifyZulipConfig,
    event: &IssuesEvent,
    inputs: Vec<NotifyZulipInput>,
) -> anyhow::Result<()> {
    for input in inputs {
        let config = &config.labels[&input.label.name];

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
    }

    Ok(())
}
