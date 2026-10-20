fn main() {
    let args = doctty::cli::parse();

    if let Err(error) = doctty::app::run(args) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
