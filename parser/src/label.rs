//! The labels command parser.
//!
//! This can parse arbitrary input, giving the list of labels added/removed.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `labels: <label-list>.`
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

#[derive(Debug, PartialEq, Eq)]
pub enum LabelDelta<'a> {
    Add(Label<'a>),
    Remove(Label<'a>),
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct Label<'a>(&'a str);

impl<'a> Label<'a> {
    fn parse(input: &'a str) -> Result<Label<'a>, ParseError<'a>> {
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
    fn parse(input: &mut TokenStream<'a>) -> Result<LabelDelta<'a>, ParseError<'a>> {
        let delta = match input.current() {
            Some(Token::Word(delta)) => {
                input.advance()?;
                delta
            }
            cur => {
                return Err(ParseError::Unexpected {
                    found: cur,
                    expected: "label delta",
                });
            }
        };
        if delta.starts_with('+') {
            Ok(LabelDelta::Add(Label::parse(&delta[1..])?))
        } else if delta.starts_with('-') {
            Ok(LabelDelta::Remove(Label(&delta[1..])))
        } else {
            Ok(LabelDelta::Add(Label::parse(delta)?))
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError<'a> {
    EmptyLabel,
    UnexpectedEnd,
    Unexpected {
        found: Option<Token<'a>>,
        expected: &'static str,
    },
}

fn parse_command<'a>(input: &mut TokenStream<'a>) -> Result<Vec<LabelDelta<'a>>, ParseError<'a>> {
    input.eat(Token::Labels, "labels command start")?;

    let mut deltas = Vec::new();

    loop {
        let delta = LabelDelta::parse(input)?;
        deltas.push(delta);

        // optional `, and` separator
        let _ = input.eat(Token::Comma, "");
        let _ = input.eat(Token::And, "");

        if let Some(Token::Dot) = input.current() {
            input.advance()?;
            break;
        }
    }

    Ok(deltas)
}

pub fn parse<'a>(input: &'a str) -> Result<Vec<LabelDelta<'a>>, ParseError<'a>> {
    let mut toks = TokenStream::new(input);
    let mut labels = Vec::new();
    while !toks.at_end() {
        if toks.current() == Some(Token::Labels) {
            match parse_command(&mut toks) {
                Ok(deltas) => {
                    labels.extend(deltas);
                }
                Err(err) => return Err(err),
            }
        } else {
            // not the labels command
            toks.advance()?;
        }
    }
    Ok(labels)
}

#[test]
fn parse_simple() {
    assert_eq!(
        parse("labels: +T-compiler -T-lang bug."),
        Ok(vec![
            LabelDelta::Add(Label("T-compiler")),
            LabelDelta::Remove(Label("T-lang")),
            LabelDelta::Add(Label("bug")),
        ])
    );
}

#[test]
fn parse_empty() {
    assert_eq!(
        TokenStream::new("labels:+,-,,,."),
        [
            Token::Labels,
            Token::Word("+"),
            Token::Comma,
            Token::Word("-"),
            Token::Comma,
            Token::Word(""),
            Token::Comma,
            Token::Word(""),
            Token::Comma,
            Token::Dot
        ]
    );
    assert_eq!(parse("labels:+,,."), Err(ParseError::EmptyLabel));
}

#[test]
fn parse_no_label_paragraph() {
    assert_eq!(
        parse("Labels do in fact exist but this is not a label paragraph."),
        Ok(vec![]),
    );
}

#[test]
fn parse_multi_label() {
    let para = "
        labels: T-compiler, T-core. However, we probably *don't* want the
        labels: -T-lang and -T-libs.
        This nolabels:bar foo should not be a label statement.
    ";
    assert_eq!(
        parse(para),
        Ok(vec![
            LabelDelta::Add(Label("T-compiler")),
            LabelDelta::Add(Label("T-core")),
            LabelDelta::Remove(Label("T-lang")),
            LabelDelta::Remove(Label("T-libs")),
        ])
    );
}

#[test]
fn parse_no_end() {
    assert_eq!(
        parse("labels: +T-compiler -T-lang bug"),
        Err(ParseError::Unexpected {
            found: None,
            expected: "label delta"
        }),
    );
}
