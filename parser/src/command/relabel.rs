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
//! <label>:
//! - \S+
//! - https://github.com/\S+/\S+/labels/\S+
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
    UnrecognizedUrl,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::EmptyLabel => write!(f, "empty label"),
            ParseError::ExpectedLabelDelta => write!(f, "a label delta"),
            ParseError::MisleadingTo => write!(f, "forbidden `to`, use `+to`"),
            ParseError::UnrecognizedUrl => write!(f, "unrecognized URL"),
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
        let mut delta = match input.peek_token()? {
            Some(Token::Word(delta)) => {
                input.next_token()?;
                delta
            }
            _ => {
                return Err(input.error(ParseError::ExpectedLabelDelta));
            }
        };

        let label_action = if delta.starts_with('+') {
            delta = &delta[1..];
            LabelDelta::Add
        } else if delta.starts_with('-') {
            delta = &delta[1..];
            LabelDelta::Remove
        } else {
            LabelDelta::Add
        };

        // Handle URLs of the form https://github.com/.../.../labels/...
        let mut urldecoded_label = None;
        if delta == "https" && input.starts_with("://") {
            let rest_of_url = input.take_until_whitespace();
            let mut pieces = rest_of_url.splitn(7, '/');
            if pieces.nth(2) == Some("github.com") && pieces.nth(2) == Some("labels") {
                if let Some(encoded_label) = pieces.next() {
                    if let Ok(decoded) = urlencoding::decode(encoded_label) {
                        urldecoded_label = Some(decoded);
                    }
                }
            }
            if let Some(urldecoded_label) = &urldecoded_label {
                delta = urldecoded_label;
            } else {
                return Err(input.error(ParseError::UnrecognizedUrl));
            }
        }

        Label::parse(delta)
            .map(label_action)
            .map_err(|e| input.error(e))
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
            toks.eat_token(Token::Word("to"))?;
            toks.eat_token(Token::Colon)?;

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
            toks.eat_token(Token::Comma)?;
            toks.eat_token(Token::Word("and"))?;

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
        parse("label to +T-compiler -T-lang bug"),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler".into())),
            LabelDelta::Remove(Label("T-lang".into())),
            LabelDelta::Add(Label("bug".into())),
        ]))
    );
}

#[test]
fn parse_shorter_command_with_to_colon() {
    assert_eq!(
        parse("label to: +T-compiler -T-lang bug"),
        Ok(Some(vec![
            LabelDelta::Add(Label("T-compiler".into())),
            LabelDelta::Remove(Label("T-lang".into())),
            LabelDelta::Add(Label("bug".into())),
        ]))
    );
}

#[test]
fn parse_delta_empty() {
    assert_eq!(
        parse("label + T-lang")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::EmptyLabel),
    );
}

#[test]
fn parse_unrecognized_url() {
    assert_eq!(
        parse("label +https://rust-lang.org")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::UnrecognizedUrl),
    );
}

#[test]
fn parse_label_url() {
    assert_eq!(
        parse("label -https://github.com/rust-lang/triagebot/labels/T-libs +https://github.com/rust-lang/triagebot/labels/T-libs-api"),
        Ok(Some(vec![
            LabelDelta::Remove(Label("T-libs".into())),
            LabelDelta::Add(Label("T-libs-api".into())),
        ])),
    );
}

#[test]
fn parse_label_url_with_percent_encoding() {
    assert_eq!(
        parse("label https://github.com/rust-lang/triagebot/labels/help%20wanted"),
        Ok(Some(vec![LabelDelta::Add(Label("help wanted".into()))])),
    );
}

#[test]
fn parse_space_before_url() {
    assert_eq!(
        parse("label + https://github.com/rust-lang/triagebot/labels/T-lang")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::EmptyLabel),
    );
}

#[test]
fn parse_space_inside_url() {
    assert_eq!(
        parse("label +https ://github.com/rust-lang/triagebot/labels/T-lang")
            .unwrap_err()
            .source()
            .unwrap()
            .downcast_ref(),
        Some(&ParseError::ExpectedLabelDelta),
    );
}
