use crate::code_block::ColorCodeBlocks;
use crate::error::Error;
use crate::token::{Token, Tokenizer};

pub mod assign;
pub mod nominate;
pub mod ping;
pub mod relabel;

pub fn find_commmand_start(input: &str, bot: &str) -> Option<usize> {
    input.find(&format!("@{}", bot))
}

#[derive(Debug)]
pub enum Command<'a> {
    Relabel(Result<relabel::RelabelCommand, Error<'a>>),
    Assign(Result<assign::AssignCommand, Error<'a>>),
    Ping(Result<ping::PingCommand, Error<'a>>),
    Nominate(Result<nominate::NominateCommand, Error<'a>>),
    None,
}

#[derive(Debug)]
pub struct Input<'a> {
    all: &'a str,
    parsed: usize,
    code: ColorCodeBlocks,
    bot: &'a str,
}

fn parse_single_command<'a, T, F, M>(
    parse: F,
    mapper: M,
    tokenizer: &Tokenizer<'a>,
) -> Option<(Tokenizer<'a>, Command<'a>)>
where
    F: FnOnce(&mut Tokenizer<'a>) -> Result<Option<T>, Error<'a>>,
    M: FnOnce(Result<T, Error<'a>>) -> Command<'a>,
    T: std::fmt::Debug,
{
    let mut tok = tokenizer.clone();
    let res = parse(&mut tok);
    log::info!("parsed {:?} command: {:?}", std::any::type_name::<T>(), res);
    match res {
        Ok(None) => None,
        Ok(Some(v)) => Some((tok, mapper(Ok(v)))),
        Err(err) => Some((tok, mapper(Err(err)))),
    }
}

impl<'a> Input<'a> {
    pub fn new(input: &'a str, bot: &'a str) -> Input<'a> {
        Input {
            all: input,
            parsed: 0,
            code: ColorCodeBlocks::new(input),
            bot,
        }
    }

    fn maybe_parse_review(&mut self) -> Option<(Tokenizer<'a>, assign::AssignCommand)> {
        // Both `r?` and `R?`.
        let start = match self.all[self.parsed..]
            .find("r?")
            .or_else(|| self.all[self.parsed..].find("R?"))
        {
            Some(pos) => pos,
            None => return None,
        };
        self.parsed += start;
        let mut tok = Tokenizer::new(&self.all[self.parsed..]);
        match tok.next_token() {
            Ok(Some(Token::Word(w))) => {
                if w != "r" && w != "R" {
                    // If we're intersecting with something else just exit
                    log::trace!("received odd review start token: {:?}", w);
                    return None;
                }
            }
            other => {
                log::trace!("received odd review start token: {:?}", other);
                return None;
            }
        }
        match tok.next_token() {
            Ok(Some(Token::Question)) => {}
            other => {
                log::trace!("received odd review start token: {:?}", other);
                return None;
            }
        }
        log::info!("identified potential review request");
        match tok.next_token() {
            Ok(Some(Token::Word(w))) => {
                let mentions = crate::mentions::get_mentions(w);
                if let [a] = &mentions[..] {
                    return Some((
                        tok,
                        assign::AssignCommand::User {
                            username: (*a).to_owned(),
                        },
                    ));
                } else {
                    log::trace!("{:?} had non-one mention: {:?}", w, mentions);
                    None
                }
            }
            other => {
                log::trace!("received odd review start token: {:?}", other);
                None
            }
        }
    }

    pub fn parse_command(&mut self) -> Command<'a> {
        let mut success = vec![];

        if let Some((tok, assign)) = self.maybe_parse_review() {
            success.push((tok, Command::Assign(Ok(assign))));
        }

        if let Some(start) = find_commmand_start(&self.all[self.parsed..], self.bot) {
            self.parsed += start;
            let mut tok = Tokenizer::new(&self.all[self.parsed..]);
            assert_eq!(
                tok.next_token().unwrap(),
                Some(Token::Word(&format!("@{}", self.bot)))
            );
            log::info!("identified potential command");
            let original_tokenizer = tok.clone();

            success.extend(parse_single_command(
                relabel::RelabelCommand::parse,
                Command::Relabel,
                &original_tokenizer,
            ));
            success.extend(parse_single_command(
                assign::AssignCommand::parse,
                Command::Assign,
                &original_tokenizer,
            ));
            success.extend(parse_single_command(
                ping::PingCommand::parse,
                Command::Ping,
                &original_tokenizer,
            ));
            success.extend(parse_single_command(
                nominate::NominateCommand::parse,
                Command::Nominate,
                &original_tokenizer,
            ));
        }

        if success.len() > 1 {
            panic!(
                "succeeded parsing {:?} to multiple commands: {:?}",
                &self.all[self.parsed..],
                success
            );
        }

        match success.pop() {
            Some((mut tok, c)) => {
                if self
                    .code
                    .overlaps_code((self.parsed)..(self.parsed + tok.position()))
                    .is_some()
                {
                    log::info!("command overlaps code; code: {:?}", self.code);
                    return Command::None;
                }
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
            Command::Relabel(r) => r.is_ok(),
            Command::Assign(r) => r.is_ok(),
            Command::Ping(r) => r.is_ok(),
            Command::Nominate(r) => r.is_ok(),
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

#[test]
fn parse_assign_review() {
    let input = "R? @user";
    let mut input = Input::new(input, "bot");
    match input.parse_command() {
        Command::Assign(Ok(x)) => assert_eq!(
            x,
            assign::AssignCommand::User {
                username: String::from("user"),
            }
        ),
        o => panic!("unknown: {:?}", o),
    };
}

#[test]
fn parse_assign_review_no_panic() {
    let input = "R ?";
    let mut input = Input::new(input, "bot");
    assert!(input.parse_command().is_none());
}
