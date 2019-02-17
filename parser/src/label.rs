//! The labels command parser.
//!
//! This can parse arbitrary input, giving the list of labels added/removed.
//!
//! The grammar is as follows:
//!
//! ```text
//! labels: <label-list>.
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

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Token<'a> {
    Labels,
    Comma,
    Dot,
    And,
    Word(&'a str),
}

impl<'a> Token<'a> {
    fn divide(self, split: char, tok: Token<'a>) -> Vec<Token<'a>> {
        let word = if let Token::Word(word) = self {
            word
        } else {
            return vec![self];
        };
        if !word.contains(split) {
            return vec![self];
        }
        let mut toks = word
            .split(split)
            .flat_map(|w| vec![Token::Word(w), tok])
            .collect::<Vec<_>>();
        // strip last token that we inserted; it's not actually one we need/want.
        assert_eq!(toks.pop(), Some(tok));
        if word.ends_with(split) {
            // strip empty string
            assert_eq!(toks.pop(), Some(Token::Word("")));
        }
        if word.starts_with(split) {
            // strip empty string
            assert_eq!(toks.remove(0), Token::Word(""));
        }
        toks
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

#[derive(Debug)]
struct TokenStream<'a> {
    tokens: Vec<Token<'a>>,
    position: usize,
}

impl<'a> TokenStream<'a> {
    fn new(input: &'a str) -> TokenStream<'a> {
        let tokens = input
            .split_whitespace()
            .map(|word| Token::Word(word))
            .flat_map(|tok| tok.divide(',', Token::Comma))
            .flat_map(|tok| tok.divide('.', Token::Dot))
            .map(|tok| {
                if let Token::Word("and") = tok {
                    Token::And
                } else {
                    tok
                }
            })
            .flat_map(|tok| {
                if let Token::Word(word) = tok {
                    let split = "labels:";
                    if word.starts_with(split) {
                        if word == split {
                            vec![Token::Labels]
                        } else {
                            vec![Token::Labels, Token::Word(&word[split.len()..])]
                        }
                    } else {
                        vec![tok]
                    }
                } else {
                    vec![tok]
                }
            })
            .collect();
        TokenStream {
            tokens,
            position: 0,
        }
    }

    fn current(&self) -> Option<Token<'a>> {
        self.tokens.get(self.position).cloned()
    }

    fn advance(&mut self) -> Result<(), ParseError<'a>> {
        self.position += 1;
        if self.position > self.tokens.len() {
            return Err(ParseError::UnexpectedEnd);
        }
        Ok(())
    }

    fn eat(&mut self, tok: Token<'a>, expect: &'static str) -> Result<(), ParseError<'a>> {
        if self.current() == Some(tok) {
            self.advance()?;
            return Ok(());
        }

        Err(ParseError::Unexpected {
            found: self.current(),
            expected: expect,
        })
    }

    fn at_end(&self) -> bool {
        self.position == self.tokens.len()
    }
}

impl<'a, T> PartialEq<T> for TokenStream<'a>
where
    T: ?Sized + PartialEq<[Token<'a>]>,
{
    fn eq(&self, other: &T) -> bool {
        other == &self.tokens[self.position..]
    }
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
fn tokenize_1() {
    assert_eq!(
        TokenStream::new("foo\t\r\n bar\nbaz"),
        [Token::Word("foo"), Token::Word("bar"), Token::Word("baz"),]
    );
}

#[test]
fn tokenize_2() {
    assert_eq!(
        TokenStream::new(",.,.,"),
        [
            Token::Comma,
            Token::Dot,
            Token::Comma,
            Token::Dot,
            Token::Comma
        ]
    );
}

#[test]
fn tokenize_whitespace_dots() {
    assert_eq!(
        TokenStream::new("baz . ,bar "),
        [
            Token::Word("baz"),
            Token::Dot,
            Token::Comma,
            Token::Word("bar")
        ]
    );
}

#[test]
fn tokenize_3() {
    assert_eq!(
        TokenStream::new("bar, and -baz"),
        [
            Token::Word("bar"),
            Token::Comma,
            Token::And,
            Token::Word("-baz"),
        ]
    );
}

#[test]
fn tokenize_labels() {
    assert_eq!(TokenStream::new("labels:"), [Token::Labels]);
    assert_eq!(
        TokenStream::new("foo labels:"),
        [Token::Word("foo"), Token::Labels]
    );
    assert_eq!(
        TokenStream::new("labels:T-compiler"),
        [Token::Labels, Token::Word("T-compiler")]
    );
    assert_eq!(
        TokenStream::new("barlabels:T-compiler"),
        [Token::Word("barlabels:T-compiler")]
    );
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
fn parse_nested_labels() {
    assert_eq!(
        parse("labels: +foo, bar, labels: oh no.."),
        Err(ParseError::Unexpected {
            found: Some(Token::Labels),
            expected: "label delta"
        }),
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
