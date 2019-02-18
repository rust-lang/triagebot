use std::fmt;
use std::iter::Peekable;
use std::str::CharIndices;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Token<'a> {
    Dot,
    Comma,
    Semi,
    Exclamation,
    Question,
    Colon,
    Quote(&'a str),
    Word(&'a str),
}

#[derive(Debug)]
pub struct Tokenizer<'a> {
    input: &'a str,
    chars: Peekable<CharIndices<'a>>,
}

#[derive(Debug, Copy, Clone)]
pub struct Error<'a> {
    input: &'a str,
    position: usize,
    kind: ErrorKind,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    UnterminatedString,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ErrorKind::UnterminatedString => "unterminated string",
            }
        )
    }
}

impl<'a> Error<'a> {
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl<'a> fmt::Display for Error<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let space = 10;
        let end = std::cmp::min(self.input.len(), self.position + space);
        write!(
            f,
            "...{}|error: {} at >|{}...",
            &self.input[self.position.saturating_sub(space)..self.position],
            self.kind,
            &self.input[self.position..end],
        )
    }
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Tokenizer<'a> {
        Tokenizer {
            input: input,
            chars: input.char_indices().peekable(),
        }
    }

    fn error(&mut self, kind: ErrorKind) -> Error<'a> {
        Error {
            input: self.input,
            position: self.cur_pos(),
            kind,
        }
    }

    fn consume_whitespace(&mut self) {
        while self.cur().map_or(false, |c| c.1.is_whitespace()) {
            self.advance();
        }
    }

    fn cur_punct(&mut self) -> Option<Token<'static>> {
        let (_, ch) = self.cur()?;
        match ch {
            '.' => Some(Token::Dot),
            ',' => Some(Token::Comma),
            ':' => Some(Token::Colon),
            '!' => Some(Token::Exclamation),
            '?' => Some(Token::Question),
            ';' => Some(Token::Semi),
            _ => None,
        }
    }

    fn consume_punct(&mut self) -> Option<Token<'a>> {
        let x = self.cur_punct()?;
        self.advance();
        Some(x)
    }

    fn cur(&mut self) -> Option<(usize, char)> {
        self.chars.peek().cloned()
    }

    fn at_end(&mut self) -> bool {
        self.chars.peek().is_none()
    }

    fn advance(&mut self) -> Option<()> {
        let (_, _) = self.chars.next()?;
        Some(())
    }

    fn cur_pos(&mut self) -> usize {
        self.cur().map_or(self.input.len(), |(pos, _)| pos)
    }

    fn str_from(&mut self, pos: usize) -> &'a str {
        &self.input[pos..self.cur_pos()]
    }

    fn consume_string(&mut self) -> Result<Option<Token<'a>>, Error<'a>> {
        if let Some((_, '"')) = self.cur() {
            // okay
        } else {
            return Ok(None);
        }
        self.advance(); // eat "
        let start = self.cur_pos();
        loop {
            match self.cur() {
                Some((_, '"')) => break,
                Some(_) => self.advance(),
                None => return Err(self.error(ErrorKind::UnterminatedString)),
            };
        }
        let body = self.str_from(start);
        self.advance(); // eat final '"'
        Ok(Some(Token::Quote(body)))
    }

    pub fn next_token(&mut self) -> Result<Option<Token<'a>>, Error<'a>> {
        self.consume_whitespace();
        if self.at_end() {
            return Ok(None);
        }
        if let Some(punct) = self.consume_punct() {
            return Ok(Some(punct));
        }

        if let Some(s) = self.consume_string()? {
            return Ok(Some(s));
        }

        // Attempt to consume a word from the input.
        // Stop if we encounter whitespace or punctuation.
        let start = self.cur_pos();
        while self.cur().map_or(false, |(_, ch)| {
            !(self.cur_punct().is_some() || ch.is_whitespace())
        }) {
            if self.cur().unwrap().1 == '"' {
                // TODO: error out -- quotes should *not* be allowed inside words
            }
            self.advance();
        }
        Ok(Some(Token::Word(&self.str_from(start))))
    }
}

#[cfg(test)]
fn tokenize<'a>(input: &'a str) -> Result<Vec<Token<'a>>, Error<'a>> {
    let mut tokens = Vec::new();
    let mut gen = Tokenizer::new(input);
    while let Some(tok) = gen.next_token()? {
        tokens.push(tok);
    }
    Ok(tokens)
}

#[test]
fn tokenize_1() {
    assert_eq!(
        tokenize("foo\t\r\n bar\nbaz").unwrap(),
        [Token::Word("foo"), Token::Word("bar"), Token::Word("baz"),]
    );
}

#[test]
fn tokenize_2() {
    assert_eq!(
        tokenize(",,,.,.,").unwrap(),
        [
            Token::Comma,
            Token::Comma,
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
        tokenize("baz . ,bar ").unwrap(),
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
        tokenize("bar, and -baz").unwrap(),
        [
            Token::Word("bar"),
            Token::Comma,
            Token::Word("and"),
            Token::Word("-baz"),
        ]
    );
}

#[test]
fn tokenize_4() {
    assert_eq!(
        tokenize(", , b").unwrap(),
        [Token::Comma, Token::Comma, Token::Word("b")]
    );
}

#[test]
fn tokenize_5() {
    assert_eq!(tokenize(r#""testing""#).unwrap(), [Token::Quote("testing")]);
}

#[test]
fn tokenize_6() {
    assert_eq!(
        tokenize(r#""testing"#).unwrap_err().kind(),
        ErrorKind::UnterminatedString
    );
}
