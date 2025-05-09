//! The glacier command parser.
//!
//! This adds the option to track ICEs. The <code-source> must be in quotes.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot glacier <code-source>`
//!
//! <code-source>: any URL that resolves to plain-text Rust code
//! ```

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use std::fmt;

#[derive(PartialEq, Eq, Debug)]
pub struct GlacierCommand {
    pub source: String,
}

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    NoLink,
    InvalidLink,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::NoLink => write!(f, "no link provided - did you forget the quotes around it?"),
            Self::InvalidLink => write!(f, "invalid link - must be from a playground gist"),
        }
    }
}

impl GlacierCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<GlacierCommand>, Error<'a>> {
        let mut toks = input.clone();
        if let Some(Token::Word("glacier")) = toks.peek_token()? {
            toks.next_token()?;
            match toks.next_token()? {
                Some(Token::Quote(s)) => {
                    let source = s.to_string();
                    if source.starts_with("https://gist.github.com/") {
                        return Ok(Some(GlacierCommand { source }));
                    } else {
                        return Err(toks.error(ParseError::InvalidLink));
                    }
                }
                Some(Token::Word(_)) => {
                    return Err(toks.error(ParseError::InvalidLink));
                }
                _ => {
                    return Err(toks.error(ParseError::NoLink));
                }
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn parse<'a>(input: &'a str) -> Result<Option<GlacierCommand>, Error<'a>> {
        let mut toks = Tokenizer::new(input);
        Ok(GlacierCommand::parse(&mut toks)?)
    }

    #[test]
    fn glacier_empty() {
        use std::error::Error;
        assert_eq!(
            parse("glacier")
                .unwrap_err()
                .source()
                .unwrap()
                .downcast_ref(),
            Some(&ParseError::NoLink),
        );
    }

    #[test]
    fn glacier_invalid() {
        use std::error::Error;
        assert_eq!(
            parse("glacier hello")
                .unwrap_err()
                .source()
                .unwrap()
                .downcast_ref(),
            Some(&ParseError::InvalidLink),
        );
    }

    #[test]
    fn glacier_valid() {
        assert_eq!(
            parse(
                r#"glacier "https://gist.github.com/rust-play/89d6c8a2398dd2dd5fcb7ef3e8109c7b""#
            ),
            Ok(Some(GlacierCommand {
                source: "https://gist.github.com/rust-play/89d6c8a2398dd2dd5fcb7ef3e8109c7b".into()
            }))
        );
    }
}
