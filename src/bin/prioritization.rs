use std::io::{self, Write};
use triagebot::prioritization::{self, Action};

#[tokio::main]
async fn main() {
    let meeting = prioritization::config::prepare_meeting();

    for step in &meeting.steps {
        println!("{}", step.call());

        //press_key_to_continue();
    }
}

fn press_key_to_continue() {
    let mut stdout = io::stdout();
    stdout
        .write(b"Press a key to continue ...")
        .expect("Unable to write to stdout");
    stdout.flush().expect("Unable to flush stdout");

    io::stdin()
        .read_line(&mut String::new())
        .expect("Unable to read user input");
}
