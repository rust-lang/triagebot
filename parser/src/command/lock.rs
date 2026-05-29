use crate::error::Error;
use crate::token::{Token, Tokenizer};

#[derive(PartialEq, Eq, Debug)]
pub enum LockCommand {
    Lock,
    Unlock,
}

impl LockCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        if let Some(Token::Word("lock")) = input.peek_token()? {
            Ok(Some(LockCommand::Lock))
        } else if let Some(Token::Word("unlock")) = input.peek_token()? {
            Ok(Some(LockCommand::Unlock))
        } else {
            Ok(None)
        }
    }
}
