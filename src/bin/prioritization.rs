use std::io::{self, Write};
use triagebot::{logger, prioritization};

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    logger::init();

    let prioritization_steps = prioritization::prepare_steps();

    for step in &prioritization_steps {
        print!("{}", step.call().await);

        press_key_to_continue();
    }
}

fn press_key_to_continue() {
    let mut stdout = io::stdout();
    stdout
        .write(b"[Press Enter to continue]")
        .expect("Unable to write to stdout");
    stdout.flush().expect("Unable to flush stdout");

    io::stdin()
        .read_line(&mut String::new())
        .expect("Unable to read user input");
}
