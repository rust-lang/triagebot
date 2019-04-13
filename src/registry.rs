use crate::github::Event;
use failure::Error;

pub struct HandleRegistry {
    handlers: Vec<Box<dyn Handler>>,
}

impl HandleRegistry {
    pub fn new() -> HandleRegistry {
        HandleRegistry {
            handlers: Vec::new(),
        }
    }

    pub fn register<H: Handler + 'static>(&mut self, h: H) {
        self.handlers.push(Box::new(h));
    }

    pub fn handle(&self, event: &Event) -> Result<(), Error> {
        let mut last_error = None;
        for h in &self.handlers {
            match h.handle_event(event) {
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
    fn handle_event(&self, event: &Event) -> Result<(), Error>;
}
