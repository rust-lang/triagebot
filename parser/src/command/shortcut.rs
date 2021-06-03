//! The shortcut command parser.
//!
//! This can parse predefined shortcut input, single word commands.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot ready`, or `@bot author`.
//! ```

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use std::fmt;

#[derive(PartialEq, Eq, Debug)]
pub enum ShortcutCommand {
    Ready,
    Author,
}

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    ExpectedEnd,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::ExpectedEnd => write!(f, "expected end of command"),
        }
    }
}

impl ShortcutCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        let mut toks = input.clone();
        if let Some(Token::Word("ready")) = toks.peek_token()? {
            toks.next_token()?;
            if let Some(Token::Dot) | Some(Token::EndOfLine) = toks.peek_token()? {
                toks.next_token()?;
                *input = toks;
                return Ok(Some(ShortcutCommand::Ready));
            } else {
                return Err(toks.error(ParseError::ExpectedEnd));
            }
        } else if let Some(Token::Word("author")) = toks.peek_token()? {
            toks.next_token()?;
            if let Some(Token::Dot) | Some(Token::EndOfLine) = toks.peek_token()? {
                toks.next_token()?;
                *input = toks;
                return Ok(Some(ShortcutCommand::Author));
            } else {
                return Err(toks.error(ParseError::ExpectedEnd));
            }
        } else {
            return Ok(None);
        }
    }
}

#[cfg(test)]
fn parse(input: &str) -> Result<Option<ShortcutCommand>, Error<'_>> {
    let mut toks = Tokenizer::new(input);
    Ok(ShortcutCommand::parse(&mut toks)?)
}

#[test]
fn test_1() {
    assert_eq!(parse("ready."), Ok(Some(ShortcutCommand::Ready)),);
}

#[test]
fn test_2() {
    assert_eq!(parse("ready"), Ok(Some(ShortcutCommand::Ready)),);
}

#[test]
fn test_3() {
    assert_eq!(parse("author"), Ok(Some(ShortcutCommand::Author)),);
}

#[test]
fn test_4() {
    use std::error::Error;
    assert_eq!(
        parse("ready word")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::ExpectedEnd),
    );
}
