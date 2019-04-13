use crate::github::Event;
use crate::handlers::Context;
use failure::Error;

pub struct HandleRegistry {
    handlers: Vec<Box<dyn Handler>>,
    ctx: Context,
}

impl HandleRegistry {
    pub fn new(ctx: Context) -> HandleRegistry {
        HandleRegistry {
            handlers: Vec::new(),
            ctx,
        }
    }

    pub fn register<H: Handler + 'static>(&mut self, h: H) {
        self.handlers.push(Box::new(h));
    }

    pub fn handle(&self, event: &Event) -> Result<(), Error> {
        let mut last_error = None;
        for h in &self.handlers {
            match h.handle_event(&self.ctx, event) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("event handling failed: {:?}", e);
                    last_error = Some(e);
                }
            }
        }
        if let Some(err) = last_error {
            Err(err)
        } else {
            Ok(())
        }
    }
}

pub trait Handler: Sync + Send {
    fn handle_event(&self, ctx: &Context, event: &Event) -> Result<(), Error>;
}
