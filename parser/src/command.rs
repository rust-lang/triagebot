use crate::code_block::ColorCodeBlocks;
use crate::error::Error;
use crate::token::{Token, Tokenizer};

pub mod label;

pub fn find_commmand_start(input: &str, bot: &str) -> Option<usize> {
    input.find(&format!("@{}", bot))
}

#[derive(Debug)]
pub enum Command<'a> {
    Label(Result<label::LabelCommand, Error<'a>>),
    None,
}

#[derive(Debug)]
pub struct Input<'a> {
    all: &'a str,
    parsed: usize,
    code: ColorCodeBlocks,
    bot: &'a str,
}

impl<'a> Input<'a> {
    pub fn new(input: &'a str, bot: &'a str) -> Input<'a> {
        Input {
            all: input,
            parsed: 0,
            code: ColorCodeBlocks::new(input),
            bot: bot,
        }
    }

    pub fn parse_command(&mut self) -> Command<'a> {
        let start = match find_commmand_start(&self.all[self.parsed..], self.bot) {
            Some(pos) => pos,
            None => return Command::None,
        };
        self.parsed += start;
        let mut tok = Tokenizer::new(&self.all[self.parsed..]);
        assert_eq!(
            tok.next_token().unwrap(),
            Some(Token::Word(&format!("@{}", self.bot)))
        );

        let mut success = vec![];

        let original_tokenizer = tok.clone();

        {
            let mut tok = original_tokenizer.clone();
            let res = label::LabelCommand::parse(&mut tok);
            match res {
                Ok(None) => {}
                Ok(Some(cmd)) => {
                    success.push((tok, Command::Label(Ok(cmd))));
                }
                Err(err) => {
                    success.push((tok, Command::Label(Err(err))));
                }
            }
        }

        if success.len() > 1 {
            panic!(
                "succeeded parsing {:?} to multiple commands: {:?}",
                &self.all[self.parsed..],
                success
            );
        }

        if let Some(_) = self
            .code
            .overlaps_code((self.parsed)..(self.parsed + tok.position()))
        {
            return Command::None;
        }

        match success.pop() {
            Some((mut tok, c)) => {
                // if we errored out while parsing the command do not move the input forwards
                if c.is_ok() {
                    self.parsed += tok.position();
                }
                c
            }
            None => Command::None,
        }
    }
}

impl<'a> Command<'a> {
    pub fn is_ok(&self) -> bool {
        match self {
            Command::Label(r) => r.is_ok(),
            Command::None => true,
        }
    }

    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }

    pub fn is_none(&self) -> bool {
        match self {
            Command::None => true,
            _ => false,
        }
    }
}

#[test]
fn errors_outside_command_are_fine() {
    let input =
        "haha\" unterminated quotes @bot modify labels: +bug. Terminating after the command";
    let mut input = Input::new(input, "bot");
    assert!(input.parse_command().is_ok());
}

#[test]
fn code_1() {
    let input = "`@bot modify labels: +bug.`";
    let mut input = Input::new(input, "bot");
    assert!(input.parse_command().is_none());
}

#[test]
fn code_2() {
    let input = "```
    @bot modify labels: +bug.
    ```";
    let mut input = Input::new(input, "bot");
    assert!(input.parse_command().is_none());
}

#[test]
fn move_input_along() {
    let input = "@bot modify labels: +bug. Afterwards, delete the world.";
    let mut input = Input::new(input, "bot");
    let parsed = input.parse_command();
    assert!(parsed.is_ok());
    assert_eq!(&input.all[input.parsed..], " Afterwards, delete the world.");
}

#[test]
fn move_input_along_1() {
    let input = "@bot modify labels\": +bug. Afterwards, delete the world.";
    let mut input = Input::new(input, "bot");
    assert!(input.parse_command().is_err());
    // don't move input along if parsing the command fails
    assert_eq!(input.parsed, 0);
}
