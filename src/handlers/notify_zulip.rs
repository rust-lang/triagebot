use crate::{
    config::NotifyZulipConfig,
    github::{self, Event},
    handlers::{Context, Handler},
};
use futures::future::{BoxFuture, FutureExt};

pub(super) struct NotifyZulipInput {
    notification_type: NotificationType,
}

pub(super) enum NotificationType {
    Labeled,
    Unlabeled,
}

pub(super) struct NotifyZulipHandler;

impl Handler for NotifyZulipHandler {
    type Input = NotifyZulipInput;
    type Config = NotifyZulipConfig;

    fn parse_input(
        &self,
        _ctx: &Context,
        event: &Event,
        config: Option<&Self::Config>,
    ) -> Result<Option<Self::Input>, String> {
        if let Event::Issue(e) = event {
            if let github::IssuesAction::Labeled | github::IssuesAction::Unlabeled = e.action {
                let applied_label = &e.label.as_ref().expect("label").name;
                if let Some(config) = config.and_then(|c| c.labels.get(applied_label)) {
                    for label in &config.required_labels {
                        let pattern =  match glob::Pattern::new(label) {
                            Ok(pattern) => pattern,
                            Err(err) => {
                                log::error!("Invalid glob pattern: {}", err);
                                continue;
                            }
                        };
                        if !e.issue.labels().iter().any(|l| pattern.matches(&l.name)) {
                            // Issue misses a required label, ignore this event
                            return Ok(None);
                        }
                    }

                    if e.action == github::IssuesAction::Labeled && config.message_on_add.is_some() {
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
        }
        Ok(None)
    }

    fn handle_input<'b>(
        &self,
        ctx: &'b Context,
        config: &'b Self::Config,
        event: &'b Event,
        input: Self::Input,
    ) -> BoxFuture<'b, anyhow::Result<()>> {
        handle_input(ctx, config, event, input).boxed()
    }
}

async fn handle_input<'a>(
    ctx: &Context,
    config: &NotifyZulipConfig,
    event: &Event,
    input: NotifyZulipInput,
) -> anyhow::Result<()> {
    let event = match event {
        Event::Issue(e) => e,
        _ => unreachable!()
    };
    let config = config.labels.get(&event.label.as_ref().unwrap().name).unwrap();

    let mut topic = config.topic.clone();
    topic = topic.replace("{number}", &event.issue.number.to_string());
    topic = topic.replace("{title}", &event.issue.title);
    topic.truncate(60); // Zulip limitation

    let mut msg = match input.notification_type {
        NotificationType::Labeled => {
            config.message_on_add.as_ref().unwrap().clone()
        }
        NotificationType::Unlabeled => {
            config.message_on_remove.as_ref().unwrap().clone()
        }
    };

    msg = msg.replace("{number}", &event.issue.number.to_string());
    msg = msg.replace("{title}", &event.issue.title);

    let zulip_req = crate::zulip::MessageApiRequest {
            type_: "stream",
            to: &config.zulip_stream.to_string(),
            topic: Some(&topic),
            content: &msg,
        };
    zulip_req.send(&ctx.github.raw()).await?;

    Ok(())
}
