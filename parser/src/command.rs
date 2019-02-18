use crate::token::{Error, Token, Tokenizer};

pub fn find_commmand_start(input: &str, bot: &str) -> Option<usize> {
    input.find(&format!("@{}", bot))
}

#[derive(Debug)]
pub enum Command {
    Label(label::LabelCommand),
}

pub fn parse_command<'a>(input: &'a str, bot: &str) -> Result<Option<Command>, Error<'a>> {
    let start = match find_commmand_start(input, bot) {
        Some(pos) => pos,
        None => return Ok(None),
    };
    let input = &input[start..];
    let mut tok = Tokenizer::new(input);
    assert_eq!(
        tok.next_token().unwrap(),
        Some(Token::Word(&format!("@{}", bot)))
    );

    let cmd = Command::Label;

    Ok(Some(cmd))
}
