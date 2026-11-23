fn main() {
    if let Err(e) = codepet_task_mcp::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
