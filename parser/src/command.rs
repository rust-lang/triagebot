use crate::token::{Error, Token, Tokenizer};

/// Returns the start of the invocation, or None if at the end of the stream
pub fn eat_until_invocation<'a>(
    tok: &mut Tokenizer<'a>,
    bot: &str,
) -> Result<Option<usize>, Error<'a>> {
    let command = format!("@{}", bot);
    while let Some(token) = tok.peek_token()? {
        match token {
            Token::Word(word) if word == command => {
                // eat invocation of bot
                let pos = tok.position();
                tok.next_token().unwrap();
                return Ok(Some(pos));
            }
            // unwrap is safe because we've successfully peeked above
            _ => {
                tok.next_token().unwrap();
            }
        }
    }
    Ok(None)
}

#[test]
fn cs_1() {
    let input = "testing @bot command";
    let mut toks = Tokenizer::new(input);
    assert_eq!(toks.peek_token().unwrap(), Some(Token::Word("testing")));
    assert_eq!(eat_until_invocation(&mut toks, "bot").unwrap(), Some(7));
    assert_eq!(toks.peek_token().unwrap(), Some(Token::Word("command")));
}

#[test]
fn cs_2() {
    let input = "@bot command";
    let mut toks = Tokenizer::new(input);
    assert_eq!(toks.peek_token().unwrap(), Some(Token::Word("@bot")));
    assert_eq!(eat_until_invocation(&mut toks, "bot").unwrap(), Some(0));
    assert_eq!(toks.peek_token().unwrap(), Some(Token::Word("command")));
}

#[test]
fn cs_3() {
    let input = "no command";
    let mut toks = Tokenizer::new(input);
    assert_eq!(eat_until_invocation(&mut toks, "bot").unwrap(), None);
}
