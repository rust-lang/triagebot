macro_rules! issue_handlers {
    ($($name:ident,)*) => {
        async fn handle_issue(
            ctx: &Context,
            event: &IssuesEvent,
            config: &Arc<Config>,
            errors: &mut Vec<HandlerError>,
        ) {
            // Process the issue handlers concurrently
            let results = futures::join!(
                $(
                    async {
                        match $name::parse_input(ctx, event, config.$name.as_ref()).await {
                            Err(err) => Err(HandlerError::Message(err)),
                            Ok(Some(input)) => {
                                if let Some(config) = &config.$name {
                                    $name::handle_input(ctx, config, event, input)
                                        .await
                                        .map_err(|e| {
                                            HandlerError::Other(e.context(format!(
                                                "error when processing {} handler",
                                                stringify!($name)
                                            )))
                                        })
                                } else {
                                    Err(HandlerError::Message(format!(
                                        "The feature `{}` is not enabled in this repository.\n\
                                        To enable it add its section in the `triagebot.toml` \
                                        in the root of the repository.",
                                        stringify!($name)
                                    )))
                                }
                            }
                            Ok(None) => Ok(())
                        }
                    }
                ),*
            );

            // Destructure the results into named variables
            let ($($name,)*) = results;

            // Push errors for each handler
            $(
                if let Err(e) = $name {
                    errors.push(e);
                }
            )*
        }
    }
}

macro_rules! command_handlers {
    ($($name:ident: $enum:ident,)*) => {
        async fn handle_command(
            ctx: &Context,
            event: &Event,
            config: &Result<Arc<Config>, ConfigurationError>,
            body: &str,
            errors: &mut Vec<HandlerError>,
        ) {
            match event {
                // always handle new PRs / issues
                Event::Issue(IssuesEvent { action: IssuesAction::Opened, .. }) => {},
                Event::Issue(IssuesEvent { action: IssuesAction::Edited, .. }) => {
                    // if the issue was edited, but we don't get a `changes[body]` diff, it means only the title was edited, not the body.
                    // don't process the same commands twice.
                    if event.comment_from().is_none() {
                        log::debug!("skipping title-only edit event");
                        return;
                    }
                },
                Event::Issue(e) => {
                    // no change in issue's body for these events, so skip
                    log::debug!("skipping event, issue was {:?}", e.action);
                    return;
                }
                Event::IssueComment(e) => {
                    match e.action {
                        IssueCommentAction::Created => {}
                        IssueCommentAction::Edited => {
                            if event.comment_from().is_none() {
                                // We are not entirely sure why this happens.
                                // Sometimes when someone posts a PR review,
                                // GitHub sends an "edited" event with no
                                // changes just before the "created" event.
                                log::debug!("skipping issue comment edit without changes");
                                return;
                            }
                        }
                        IssueCommentAction::Deleted => {
                            // don't execute commands again when comment is deleted
                            log::debug!("skipping event, comment was {:?}", e.action);
                            return;
                        }
                    }
                }
                Event::Push(_) | Event::Create(_) => {
                    log::debug!("skipping unsupported event");
                    return;
                }
            }

            let input = Input::new(&body, vec![&ctx.username, "triagebot"]);
            let commands = if let Some(previous) = event.comment_from() {
                let prev_commands = Input::new(&previous, vec![&ctx.username, "triagebot"]).collect::<Vec<_>>();
                input.filter(|cmd| !prev_commands.contains(cmd)).collect::<Vec<_>>()
            } else {
                input.collect()
            };

            log::info!("Comment parsed to {:?}", commands);

            if commands.is_empty() {
                return;
            }

            let config = match config {
                Ok(config) => config,
                Err(e @ ConfigurationError::Missing) => {
                    // r? is conventionally used to mean "hey, can you review"
                    // even if the repo doesn't have a triagebot.toml. In that
                    // case, just ignore it.
                    if commands
                        .iter()
                        .all(|cmd| matches!(cmd, Command::Assign(Ok(AssignCommand::RequestReview { .. }))))
                    {
                        return;
                    }
                    return errors.push(HandlerError::Message(e.to_string()));
                }
                Err(e @ ConfigurationError::Toml(_)) => {
                    return errors.push(HandlerError::Message(e.to_string()));
                }
                Err(e @ ConfigurationError::Http(_)) => {
                    return errors.push(HandlerError::Other(e.clone().into()));
                }
            };

            for command in commands {
                match command {
                    $(
                    Command::$enum(Ok(command)) => {
                        if let Some(config) = &config.$name {
                            $name::handle_command(ctx, config, event, command)
                                .await
                                .unwrap_or_else(|mut err| {
                                    if let Some(err) = err.downcast_mut::<UserError>() {
                                        errors.push(HandlerError::Message(std::mem::take(&mut err.0)));
                                    } else {
                                        errors.push(HandlerError::Other(err.context(format!(
                                            "error when processing {} command handler",
                                            stringify!($name)
                                        ))));
                                    }
                                });
                        } else {
                            errors.push(HandlerError::Message(format!(
                                "The feature `{}` is not enabled in this repository.\n\
                                To enable it add its section in the `triagebot.toml` \
                                in the root of the repository.",
                                stringify!($name)
                            )));
                        }
                    }
                    Command::$enum(Err(err)) => {
                        errors.push(HandlerError::Message(format!(
                            "Parsing {} command in [comment]({}) failed: {}",
                            stringify!($name),
                            event.html_url().expect("has html url"),
                            err
                        )));
                    })*
                }
            }
        }
    }
}

macro_rules! custom_handlers {
    ($errors:ident -> $($name:ident: $hd:expr,)*) => {{
        // Process the handlers concurrently
        let results = futures::join!(
            $(
                async {
                    async {
                        $hd
                    }
                    .await
                    .map_err(|e: anyhow::Error| {
                        HandlerError::Other(e.context(format!(
                            "error when processing {} handler",
                            stringify!($name)
                        )))
                    })
                }
            ),*
        );

        // Destructure the results into named variables
        let ($($name,)*) = results;

        // Push errors for each handler
        $(
            if let Err(e) = $name {
                $errors.push(e);
            }
        )*
    }}
}
