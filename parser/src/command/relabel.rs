//! The labels command parser.
//!
//! This can parse arbitrary input, giving the list of labels added/removed.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot modify? <label-w> to? :? <label-list>.`
//!
//! <label-w>:
//!  - label
//!  - labels
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

#[derive(Debug, PartialEq, Eq)]
pub struct RelabelCommand(pub Vec<LabelDelta>);

#[derive(Debug, PartialEq, Eq)]
pub enum LabelDelta {
    Add(Label),
    Remove(Label),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Label(String);

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
            ParseError::MisleadingTo => write!(f, "forbidden `to`, use `+to`"),
            ParseError::NoSeparator => write!(f, "must have `:` or `to` as label starter"),
        }
    }
}

impl Label {
    fn parse(input: &str) -> Result<Label, ParseError> {
        if input.is_empty() {
            Err(ParseError::EmptyLabel)
        } else {
            Ok(Label(input.into()))
        }
    }
}

impl std::ops::Deref for Label {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

impl LabelDelta {
    fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<LabelDelta, Error<'a>> {
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

    pub fn label(&self) -> &Label {
        match self {
            LabelDelta::Add(l) => l,
            LabelDelta::Remove(l) => l,
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

impl RelabelCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        let mut toks = input.clone();

        toks.eat_token(Token::Word("modify"))?;

        if toks.eat_token(Token::Word("labels"))? || toks.eat_token(Token::Word("label"))? {
            if toks.eat_token(Token::Colon)? {
                // ate the colon
            } else if toks.eat_token(Token::Word("to"))? {
                // optionally eat the colon after to, e.g.:
                // @rustbot modify labels to: -S-waiting-on-author, +S-waiting-on-review
                toks.eat_token(Token::Colon)?;
            } else {
                // It's okay if there's no to or colon, we can just eat labels
                // afterwards.
            }
            // continue
        } else {
            return Ok(None);
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

            if let Some(Token::Semi) | Some(Token::Dot) | Some(Token::EndOfLine) =
                toks.peek_token()?
            {
                toks.next_token()?;
                *input = toks;
                return Ok(Some(RelabelCommand(deltas)));
            }
        }
    }
}

#[cfg(test)]
fn parse<'a>(input: &'a str) -> Result<Option<Vec<LabelDelta>>, Error<'a>> {
    let mut toks = Tokenizer::new(input);
    Ok(RelabelCommand::parse(&mut toks)?.map(|c| c.0))
}

#[test]
fn parse_simple() {
    assert_eq!(
        parse("modify labels: +T-compiler -T-lang bug."),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler".into())),
            LabelDelta::Remove(Label("T-lang".into())),
            LabelDelta::Add(Label("bug".into())),
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
        parse("modify labels yep; Labels do in fact exist but this is not a label paragraph."),
        Ok(Some(vec![LabelDelta::Add(Label("yep".into())),]))
    );
}

#[test]
fn parse_no_dot() {
    assert_eq!(
        parse("modify labels to +T-compiler -T-lang bug"),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler".into())),
            LabelDelta::Remove(Label("T-lang".into())),
            LabelDelta::Add(Label("bug".into())),
        ]))
    );
}

#[test]
fn parse_to_colon() {
    assert_eq!(
        parse("modify labels to: +T-compiler -T-lang bug"),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler".into())),
            LabelDelta::Remove(Label("T-lang".into())),
            LabelDelta::Add(Label("bug".into())),
        ]))
    );
}

#[test]
fn parse_shorter_command() {
    assert_eq!(
        parse("label +T-compiler -T-lang bug"),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler".into())),
            LabelDelta::Remove(Label("T-lang".into())),
            LabelDelta::Add(Label("bug".into())),
        ]))
    );
}

#[test]
fn parse_shorter_command_with_colon() {
    assert_eq!(
        parse("labels: +T-compiler -T-lang bug"),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler".into())),
            LabelDelta::Remove(Label("T-lang".into())),
            LabelDelta::Add(Label("bug".into())),
        ]))
    );
}

#[test]
fn parse_shorter_command_with_to() {
    assert_eq!(
        parse("label to +T-compiler -T-lang bug")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::MisleadingTo)
    );
}

#[test]
fn parse_shorter_command_with_to_colon() {
    assert_eq!(
        parse("label to: +T-compiler -T-lang bug")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::MisleadingTo)
    );
}
