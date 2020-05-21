//! The glacier command parser.
//!
//! This adds the option to track ICEs.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot glacier <code-source>`
//!
//! <code-source>:
//!   - "https://play.rust-lang.org/.*"
//! ```

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use std::fmt;

#[derive(PartialEq, Eq, Debug)]
pub struct GlacierCommand {
    source: String,
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
            Self::NoLink => write!(f, "no link provided"),
            Self::InvalidLink => write!(f, "invalid link"),
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
                    let source = s.to_owned();
                    if source.starts_with("https://play.rust-lang.org/") {
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
            parse(r#"glacier "https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=a85913678bee64a3262db9a4a59463c2""#),
            Ok(Some(GlacierCommand {
                source: "https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=a85913678bee64a3262db9a4a59463c2".into()
            }))
        );
    }
}
