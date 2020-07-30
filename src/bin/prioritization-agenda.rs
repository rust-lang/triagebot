use triagebot::{agenda, logger};

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    logger::init();

    let agenda = agenda::prioritization();

    print!("{}", agenda.call().await);
}
