use crate::error::Error;
use crate::token::{Token, Tokenizer};

#[derive(PartialEq, Eq, Debug)]
pub enum MergeCommand {
    Merge,
    DelegateToAuthor,
    Delegate { login: String },
}

impl MergeCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        if let Some(Token::Word("merge")) = input.peek_token()? {
            Ok(Some(MergeCommand::Merge))
        } else if let Some(Token::Word("delegate" | "delegate+")) = input.peek_token()? {
            Ok(Some(MergeCommand::DelegateToAuthor))
        } else if let Some(Token::Word(word)) = input.peek_token()?
            && let Some(login) = word.strip_prefix("delegate=")
        {
            Ok(Some(MergeCommand::Delegate {
                login: login.to_string(),
            }))
        } else {
            Ok(None)
        }
    }
}
