use crate::error::Error;
use crate::token::{Token, Tokenizer};

pub mod label;

pub fn find_commmand_start(input: &str, bot: &str) -> Option<usize> {
    input.find(&format!("@{}", bot))
}

#[derive(Debug)]
pub enum Command<'a> {
    Label(label::LabelCommand<'a>),
}

pub fn parse_command<'a>(input: &mut &'a str, bot: &str) -> Result<Option<Command<'a>>, Error<'a>> {
    let start = match find_commmand_start(input, bot) {
        Some(pos) => pos,
        None => return Ok(None),
    };
    *input = &input[start..];
    let mut tok = Tokenizer::new(&input);
    assert_eq!(
        tok.next_token().unwrap(),
        Some(Token::Word(&format!("@{}", bot)))
    );

    let mut success = vec![];

    {
        let mut lc = tok.clone();
        let res = label::LabelCommand::parse(&mut lc)?;
        match res {
            None => {}
            Some(cmd) => {
                // save tokenizer off
                tok = lc;
                success.push(Command::Label(cmd));
            }
        }
    }

    if success.len() > 1 {
        panic!(
            "succeeded parsing {:?} to multiple commands: {:?}",
            input, success
        );
    }

    // XXX: Check that command did not intersect with code block

    *input = &input[tok.position()..];

    Ok(success.pop())
}
