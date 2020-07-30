use triagebot::{logger, prioritization};

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    logger::init();

    let agenda = prioritization::prepare_agenda();

    print!("{}", agenda.call().await);
}
