//! The decision process command parser.
//!
//! This can parse arbitrary input, giving the command with which we would like
//! to vote that will potentially change the issue in its resolution,
//! reversibility and/or more.
//!
//! In the first one, we must also assign a valid team to the issue decision process.
//!
//! The grammar is as follows:
//!
//! ```text
//! Command: `@bot merge`, `@bot hold`, `@bot close`
//!
//! First comment: `@bot merge lang`, `@bot hold lang`
//! ```

use std::fmt;

use crate::error::Error;
use crate::token::{Token, Tokenizer};
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};

/// A command as parsed and received from calling the bot with some arguments,
/// like `@rustbot merge`
#[derive(Debug, Eq, PartialEq)]
pub struct DecisionCommand {
    pub resolution: Resolution,
    pub reversibility: Reversibility,
    pub team: Option<String>,
}

impl DecisionCommand {
    pub fn parse<'a>(input: &mut Tokenizer<'a>) -> Result<Option<Self>, Error<'a>> {
        let mut toks = input.clone();

        match toks.peek_token()? {
            Some(Token::Word("merge")) => {
                toks.next_token()?;

                let team: Option<String> = get_team(&mut toks)?;

                command_or_error(
                    input,
                    &mut toks,
                    Self {
                        resolution: Resolution::Merge,
                        reversibility: Reversibility::Reversible,
                        team,
                    },
                )
            }
            Some(Token::Word("hold")) => {
                toks.next_token()?;

                let team: Option<String> = get_team(&mut toks)?;

                command_or_error(
                    input,
                    &mut toks,
                    Self {
                        resolution: Resolution::Hold,
                        reversibility: Reversibility::Reversible,
                        team,
                    },
                )
            }
            _ => Ok(None),
        }
    }
}

fn get_team<'a>(toks: &mut Tokenizer<'a>) -> Result<Option<String>, Error<'a>> {
    match toks.peek_token()? {
        Some(Token::Word(team)) => {
            toks.next_token()?;

            Ok(Some(team.to_string()))
        }
        _ => Ok(None),
    }
}

fn command_or_error<'a>(
    input: &mut Tokenizer<'a>,
    toks: &mut Tokenizer<'a>,
    command: DecisionCommand,
) -> Result<Option<DecisionCommand>, Error<'a>> {
    if let Some(Token::Dot) | Some(Token::EndOfLine) = toks.peek_token()? {
        *input = toks.clone();
        Ok(Some(command))
    } else {
        Err(toks.error(ParseError::ExpectedEnd))
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    ExpectedEnd,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::ExpectedEnd => write!(f, "expected end of command"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, ToSql, FromSql, Eq, PartialEq)]
#[postgres(name = "reversibility")]
pub enum Reversibility {
    #[postgres(name = "reversible")]
    Reversible,
    #[postgres(name = "irreversible")]
    Irreversible,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, ToSql, FromSql, Eq, PartialEq)]
#[postgres(name = "resolution")]
pub enum Resolution {
    #[postgres(name = "merge")]
    Merge,
    #[postgres(name = "hold")]
    Hold,
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Resolution::Merge => write!(f, "merge"),
            Resolution::Hold => write!(f, "hold"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<'a>(input: &'a str) -> Result<Option<DecisionCommand>, Error<'a>> {
        let mut toks = Tokenizer::new(input);
        Ok(DecisionCommand::parse(&mut toks)?)
    }

    #[test]
    fn test_correct_merge() {
        assert_eq!(
            parse("merge"),
            Ok(Some(DecisionCommand {
                resolution: Resolution::Merge,
                reversibility: Reversibility::Reversible,
                team: None
            })),
        );
    }

    #[test]
    fn test_correct_merge_final_dot() {
        assert_eq!(
            parse("merge."),
            Ok(Some(DecisionCommand {
                resolution: Resolution::Merge,
                reversibility: Reversibility::Reversible,
                team: None
            })),
        );
    }

    #[test]
    fn test_correct_hold() {
        assert_eq!(
            parse("hold"),
            Ok(Some(DecisionCommand {
                resolution: Resolution::Hold,
                reversibility: Reversibility::Reversible,
                team: None
            })),
        );
    }

    #[test]
    fn test_expected_end() {
        use std::error::Error;
        assert_eq!(
            parse("hold lang beer")
                .unwrap_err()
                .source()
                .unwrap()
                .downcast_ref(),
            Some(&ParseError::ExpectedEnd),
        );
    }

    #[test]
    fn test_correct_merge_with_team() {
        assert_eq!(
            parse("merge lang"),
            Ok(Some(DecisionCommand {
                resolution: Resolution::Merge,
                reversibility: Reversibility::Reversible,
                team: Some("lang".to_string())
            })),
        );
    }

    #[test]
    fn test_correct_hold_with_team() {
        assert_eq!(
            parse("hold lang"),
            Ok(Some(DecisionCommand {
                resolution: Resolution::Hold,
                reversibility: Reversibility::Reversible,
                team: Some("lang".to_string())
            })),
        );
    }
}
