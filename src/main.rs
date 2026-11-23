use doctty::support::bench_trace;
use serde_json::json;

fn main() {
    bench_trace::init("cli");
    bench_trace::event("cli_start", json!({ "pid": std::process::id() }));
    let args = doctty::cli::parse();

    let result = doctty::app::run(args);
    bench_trace::event("cli_exit", json!({ "ok": result.is_ok() }));
    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
