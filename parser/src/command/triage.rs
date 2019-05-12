//! The triage command parser.
//!
//! Gives the priority to be changed to.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot triage {high,medium,low,P-high,P-medium,P-low}`
//! ```

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use std::fmt;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Priority {
    High,
    Low,
    Medium,
}

#[derive(PartialEq, Eq, Debug)]
pub struct TriageCommand {
    pub priority: Priority,
}

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    ExpectedPriority,
    ExpectedEnd,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::ExpectedPriority => write!(f, "expected priority (high, medium, low)"),
            ParseError::ExpectedEnd => write!(f, "expected end of command"),
        }
    }
}

impl TriageCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        let mut toks = input.clone();
        if let Some(Token::Word("triage")) = toks.peek_token()? {
            toks.next_token()?;
            let priority = match toks.peek_token()? {
                Some(Token::Word("high")) | Some(Token::Word("P-high")) => {
                    toks.next_token()?;
                    Priority::High
                }
                Some(Token::Word("medium")) | Some(Token::Word("P-medium")) => {
                    toks.next_token()?;
                    Priority::Medium
                }
                Some(Token::Word("low")) | Some(Token::Word("P-low")) => {
                    toks.next_token()?;
                    Priority::Low
                }
                _ => {
                    return Err(toks.error(ParseError::ExpectedPriority));
                }
            };
            if let Some(Token::Dot) | Some(Token::EndOfLine) = toks.peek_token()? {
                toks.next_token()?;
                *input = toks;
                return Ok(Some(TriageCommand { priority: priority }));
            } else {
                return Err(toks.error(ParseError::ExpectedEnd));
            }
        } else {
            return Ok(None);
        }
    }
}
