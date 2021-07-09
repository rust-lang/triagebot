use triagebot::{agenda, logger};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    dotenv::dotenv().ok();
    logger::init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        match &args[1][..] {
            "agenda" => {
                let agenda = agenda::lang();
                print!("{}", agenda.call().await);
                return;
            }
            "planning" => {
                let agenda = agenda::lang_planning();
                print!("{}", agenda.call().await);
                return;
            }
            _ => {}
        }
    }

    eprintln!("Usage: lang (agenda|planning)")
}
