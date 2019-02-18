//! The labels command parser.
//!
//! This can parse arbitrary input, giving the list of labels added/removed.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot modify labels:? to? <label-list>.`
//!
//! <label-list>:
//!  - <label-delta>
//!  - <label-delta> and <label-list>
//!  - <label-delta>, <label-list>
//!  - <label-delta>, and <label-list>
//!
//! <label-delta>:
//!  - +<label>
//!  - -<label>
//!  this can start with a + or -, but then the only supported way of adding it
//!  is with the previous two variants of this (i.e., ++label and -+label).
//!  - <label>
//!
//! <label>: \S+
//! ```

use crate::error::Error;
use crate::token::{Token, Tokenizer};
#[cfg(test)]
use std::error::Error as _;
use std::fmt;

#[derive(Debug)]
pub struct LabelCommand<'a>(Vec<LabelDelta<'a>>);

#[derive(Debug, PartialEq, Eq)]
pub enum LabelDelta<'a> {
    Add(Label<'a>),
    Remove(Label<'a>),
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct Label<'a>(&'a str);

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    EmptyLabel,
    ExpectedLabelDelta,
    MisleadingTo,
    NoSeparator,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::EmptyLabel => write!(f, "empty label"),
            ParseError::ExpectedLabelDelta => write!(f, "a label delta"),
            ParseError::MisleadingTo => write!(f, "forbidden to, use +to"),
            ParseError::NoSeparator => write!(f, "must have : or to as label starter"),
        }
    }
}

impl<'a> Label<'a> {
    fn parse(input: &'a str) -> Result<Label<'a>, ParseError> {
        if input.is_empty() {
            Err(ParseError::EmptyLabel)
        } else {
            Ok(Label(input))
        }
    }

    pub fn as_str(&self) -> &'a str {
        self.0
    }
}

impl<'a> std::ops::Deref for Label<'a> {
    type Target = str;
    fn deref(&self) -> &str {
        self.0
    }
}

impl<'a> LabelDelta<'a> {
    fn parse(input: &mut Tokenizer<'a>) -> Result<LabelDelta<'a>, Error<'a>> {
        let delta = match input.peek_token()? {
            Some(Token::Word(delta)) => {
                input.next_token()?;
                delta
            }
            _ => {
                return Err(input.error(ParseError::ExpectedLabelDelta));
            }
        };
        if delta.starts_with('+') {
            Ok(LabelDelta::Add(
                Label::parse(&delta[1..]).map_err(|e| input.error(e))?,
            ))
        } else if delta.starts_with('-') {
            Ok(LabelDelta::Remove(
                Label::parse(&delta[1..]).map_err(|e| input.error(e))?,
            ))
        } else {
            Ok(LabelDelta::Add(
                Label::parse(delta).map_err(|e| input.error(e))?,
            ))
        }
    }
}

#[test]
fn delta_empty() {
    let mut tok = Tokenizer::new("+ testing");
    let err = LabelDelta::parse(&mut tok).unwrap_err();
    assert_eq!(
        err.source().unwrap().downcast_ref::<ParseError>(),
        Some(&ParseError::EmptyLabel)
    );
    assert_eq!(err.position(), 1);
}

impl<'a> LabelCommand<'a> {
    pub fn parse(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        let mut toks = input.clone();
        if let Some(Token::Word("modify")) = toks.next_token()? {
            // continue
        } else {
            return Ok(None);
        }
        if let Some(Token::Word("labels")) = toks.next_token()? {
            // continue
        } else {
            return Ok(None);
        }
        if let Some(Token::Colon) = toks.peek_token()? {
            toks.next_token()?;
        } else if let Some(Token::Word("to")) = toks.peek_token()? {
            toks.next_token()?;
        } else {
            return Err(toks.error(ParseError::NoSeparator));
        }
        if let Some(Token::Word("to")) = toks.peek_token()? {
            return Err(toks.error(ParseError::MisleadingTo));
        }
        // start parsing deltas
        let mut deltas = Vec::new();
        loop {
            deltas.push(LabelDelta::parse(&mut toks)?);

            // optional `, and` separator
            if let Some(Token::Comma) = toks.peek_token()? {
                toks.next_token()?;
            }
            if let Some(Token::Word("and")) = toks.peek_token()? {
                toks.next_token()?;
            }

            if let Some(Token::Dot) = toks.peek_token()? {
                toks.next_token()?;
                *input = toks;
                return Ok(Some(LabelCommand(deltas)));
            }
        }
    }
}

#[cfg(test)]
fn parse<'a>(input: &'a str) -> Result<Option<Vec<LabelDelta<'a>>>, Error<'a>> {
    let mut toks = Tokenizer::new(input);
    Ok(LabelCommand::parse(&mut toks)?.map(|c| c.0))
}

#[test]
fn parse_simple() {
    assert_eq!(
        parse("modify labels: +T-compiler -T-lang bug."),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler")),
            LabelDelta::Remove(Label("T-lang")),
            LabelDelta::Add(Label("bug")),
        ]))
    );
}

#[test]
fn parse_leading_to_label() {
    assert_eq!(
        parse("modify labels: to -T-lang")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::MisleadingTo)
    );
}

#[test]
fn parse_no_label_paragraph() {
    assert_eq!(
        parse("modify labels yep; Labels do in fact exist but this is not a label paragraph.")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::NoSeparator)
    );
    assert_eq!(
        parse("Labels do in fact exist but this is not a label paragraph."),
        Ok(None),
    );
}

#[test]
fn parse_no_end() {
    assert_eq!(
        parse("modify labels to +T-compiler -T-lang bug")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::ExpectedLabelDelta),
    );
}
