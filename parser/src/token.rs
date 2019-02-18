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

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Tokenizer<'a> {
        Tokenizer {
            input: input,
            chars: input.char_indices().peekable(),
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

    fn consume_string(&mut self) -> Option<Token<'a>> {
        if self.cur()?.1 == '"' {
            self.advance(); // eat "
            let start = self.cur_pos();
            while self.cur().map_or(false, |c| c.1 != '"') {
                if self.advance().is_none() {
                    // XXX: Error: unterminated string
                }
            }
            let body = self.str_from(start);
            self.advance(); // eat final '"'
            return Some(Token::Quote(body));
        }
        None
    }

    pub fn next_token(&mut self) -> Option<Token<'a>> {
        self.consume_whitespace();
        if self.at_end() {
            return None;
        }
        if let Some(punct) = self.consume_punct() {
            return Some(punct);
        }

        if let Some(s) = self.consume_string() {
            return Some(s);
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
        Some(Token::Word(&self.str_from(start)))
    }
}

#[cfg(test)]
#[derive(Debug)]
struct TokenStream<'a> {
    tokens: Vec<Token<'a>>,
}

#[cfg(test)]
impl<'a> TokenStream<'a> {
    fn new(input: &'a str) -> TokenStream<'a> {
        let mut tokens = Vec::new();
        let mut gen = Tokenizer::new(input);
        while let Some(tok) = gen.next_token() {
            tokens.push(tok);
        }
        TokenStream { tokens }
    }
}

#[cfg(test)]
impl<'a, T> PartialEq<T> for TokenStream<'a>
where
    T: ?Sized + PartialEq<[Token<'a>]>,
{
    fn eq(&self, other: &T) -> bool {
        other == &self.tokens[..]
    }
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
        TokenStream::new(",,,.,.,"),
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
            Token::Word("and"),
            Token::Word("-baz"),
        ]
    );
}

#[test]
fn tokenize_4() {
    assert_eq!(
        TokenStream::new(", , b"),
        [Token::Comma, Token::Comma, Token::Word("b")]
    );
}

#[test]
fn tokenize_5() {
    assert_eq!(TokenStream::new(r#""testing""#), [Token::Quote("testing")],);
}
