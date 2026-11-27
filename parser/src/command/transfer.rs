//! Parses the `@bot transfer reponame` command.

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub struct TransferCommand(pub String);

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    MissingRepo,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::MissingRepo => write!(f, "missing repository name"),
        }
    }
}

impl TransferCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        if !matches!(input.peek_token()?, Some(Token::Word("transfer"))) {
            return Ok(None);
        }
        input.next_token()?;

        let repo = input.take_line()?.trim();
        if repo.is_empty() {
            return Err(input.error(ParseError::MissingRepo));
        }

        Ok(Some(TransferCommand(repo.to_owned())))
    }
}

#[cfg(test)]
fn parse(input: &str) -> Result<Option<String>, ParseError> {
    let mut toks = Tokenizer::new(input);
    match TransferCommand::parse(&mut toks) {
        Ok(Some(TransferCommand(c))) => Ok(Some(c)),
        Ok(None) => Ok(None),
        Err(e) => Err(*e.source.downcast().unwrap()),
    }
}

#[test]
fn parse_rust() {
    assert_eq!(parse("transfer rust"), Ok(Some("rust".into())));
}

#[test]
fn parse_www_rust_lang_org() {
    assert_eq!(
        parse("transfer www.rust-lang.org"),
        Ok(Some("www.rust-lang.org".into()))
    );
}

#[test]
fn parse_unfortunate_side_effect() {
    assert_eq!(
        parse("transfer rust please"),
        Ok(Some("rust please".into()))
    );
}

#[test]
fn parse_missing() {
    assert_eq!(parse("transfer "), Err(ParseError::MissingRepo));
}
