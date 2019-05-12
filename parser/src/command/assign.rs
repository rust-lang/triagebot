//! The assignment command parser.
//!
//! This can parse arbitrary input, giving the user to be assigned.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot claim`, `@bot release-assignment`, or `@bot assign @user`.
//! ```

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use std::fmt;

#[derive(PartialEq, Eq, Debug)]
pub enum AssignCommand {
    Own,
    Release,
    User { username: String },
}

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    ExpectedEnd,
    MentionUser,
    NoUser,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::MentionUser => write!(f, "user should start with @"),
            ParseError::ExpectedEnd => write!(f, "expected end of command"),
            ParseError::NoUser => write!(f, "specify user to assign to"),
        }
    }
}

impl AssignCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        let mut toks = input.clone();
        if let Some(Token::Word("claim")) = toks.peek_token()? {
            toks.next_token()?;
            if let Some(Token::Dot) | Some(Token::EndOfLine) = toks.peek_token()? {
                toks.next_token()?;
                *input = toks;
                return Ok(Some(AssignCommand::Own));
            } else {
                return Err(toks.error(ParseError::ExpectedEnd));
            }
        } else if let Some(Token::Word("assign")) = toks.peek_token()? {
            toks.next_token()?;
            if let Some(Token::Word(user)) = toks.next_token()? {
                if user.starts_with("@") && user.len() != 1 {
                    Ok(Some(AssignCommand::User {
                        username: user[1..].to_owned(),
                    }))
                } else {
                    return Err(toks.error(ParseError::MentionUser));
                }
            } else {
                return Err(toks.error(ParseError::NoUser));
            }
        } else if let Some(Token::Word("release-assignment")) = toks.peek_token()? {
            toks.next_token()?;
            if let Some(Token::Dot) | Some(Token::EndOfLine) = toks.peek_token()? {
                toks.next_token()?;
                *input = toks;
                return Ok(Some(AssignCommand::Release));
            } else {
                return Err(toks.error(ParseError::ExpectedEnd));
            }
        } else {
            return Ok(None);
        }
    }
}

#[cfg(test)]
fn parse<'a>(input: &'a str) -> Result<Option<AssignCommand>, Error<'a>> {
    let mut toks = Tokenizer::new(input);
    Ok(AssignCommand::parse(&mut toks)?)
}

#[test]
fn test_1() {
    assert_eq!(parse("claim."), Ok(Some(AssignCommand::Own)),);
}

#[test]
fn test_2() {
    assert_eq!(parse("claim"), Ok(Some(AssignCommand::Own)),);
}

#[test]
fn test_3() {
    assert_eq!(
        parse("assign @user"),
        Ok(Some(AssignCommand::User {
            username: "user".to_owned()
        })),
    );
}

#[test]
fn test_4() {
    use std::error::Error;
    assert_eq!(
        parse("assign @")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::MentionUser),
    );
}
