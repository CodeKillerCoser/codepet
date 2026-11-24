fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = if args.first().map(String::as_str) == Some("--http") {
        codepet_task_mcp::http::run_cli(args.get(1).map(String::as_str))
    } else {
        codepet_task_mcp::run()
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
