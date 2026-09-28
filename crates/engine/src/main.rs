mod bench;
mod datagen;
mod nnue;
mod search;
mod tt;
mod uci;
mod web;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("bench") => {
            let depth = args.next().and_then(|d| d.parse().ok()).unwrap_or(bench::DEFAULT_DEPTH);
            bench::run(depth);
        }
        Some("datagen") => match datagen::Config::parse(args) {
            Ok(config) => {
                if let Err(e) = datagen::run(&config) {
                    eprintln!("datagen: {e}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("{e}\n{}", datagen::USAGE);
                std::process::exit(2);
            }
        },
        Some("serve") => {
            let port = args.next().and_then(|p| p.parse().ok()).unwrap_or(8099);
            if let Err(e) = web::run(port) {
                eprintln!("serve: {e}");
                std::process::exit(1);
            }
        }
        _ => uci::run(),
    }
}
