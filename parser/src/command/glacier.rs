//! The glacier command parser.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command:
//!   - `@bot glacier add <code-source>?`
//!   - `@bot glacier remove`
//!
//! <code-source>:
//!   - "https://play.rust-lang.org/.*"
//! ```

use std::fmt;

use crate::error::Error;
use crate::token::{Token, Tokenizer};

#[derive(Debug, PartialEq)]
pub enum GlacierCommand {
    Remove,
    Add(CodeSource),
}

#[derive(Debug, PartialEq)]
pub enum CodeSource {
    Post,
    Url(String),
}

#[derive(Debug)]
pub enum ParseError {
    UnknownSubCommand,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::UnknownSubCommand => write!(f, "unknown command"),
        }
    }
}

impl GlacierCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<GlacierCommand>, Error<'a>> {
        let mut toks = input.clone();
        if let Some(Token::Word("glacier")) = toks.next_token()? {
            if let Some(Token::Word("add")) = toks.peek_token()? {
                toks.next_token()?;
                Ok(Some(Self::Add(
                    if let Some(Token::Quote(url)) = toks.next_token()? {
                        CodeSource::Url(url.into())
                    } else {
                        CodeSource::Post
                    },
                )))
            } else if let Some(Token::Word("remove")) = toks.peek_token()? {
                Ok(Some(Self::Remove))
            } else {
                Err(toks.error(ParseError::UnknownSubCommand))
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn parse<'a>(input: &'a str) -> Result<Option<GlacierCommand>, Error<'a>> {
        let mut toks = Tokenizer::new(input);
        Ok(GlacierCommand::parse(&mut toks)?)
    }

    #[test]
    fn test_remove() {
        assert_eq!(parse("glacier remove"), Ok(Some(GlacierCommand::Remove)));
    }

    #[test]
    fn test_add_post() {
        assert_eq!(
            parse("glacier add"),
            Ok(Some(GlacierCommand::Add(CodeSource::Post)))
        );
    }

    #[test]
    fn test_add_url() {
        assert_eq!(
            parse(r#"glacier add "https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=a85913678bee64a3262db9a4a59463c2""#),
            Ok(Some(GlacierCommand::Add(CodeSource::Url("https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=a85913678bee64a3262db9a4a59463c2".into()))))
        );
    }
}
