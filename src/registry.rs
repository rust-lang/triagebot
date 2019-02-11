use crate::IssueCommentEvent;
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
        for h in &self.handlers {
            match h.handle_event(event) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("event handling failed: {:?}", e);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum Event {
    IssueComment(IssueCommentEvent),
}

pub trait Handler: Sync + Send {
    fn handle_event(&self, event: &Event) -> Result<(), Error>;
}
