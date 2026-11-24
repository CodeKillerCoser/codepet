fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = match args.first().map(String::as_str) {
        Some("--http") => codepet_task_mcp::http::run_cli(args.get(1).map(String::as_str)),
        Some("--install") if args.len() == 1 => install(),
        Some("--managed-stdio") if args.len() == 1 => match codepet_task_mcp::managed::launch() {
            Ok(code) => std::process::exit(code),
            Err(error) => Err(error.into()),
        },
        Some("--data-dir") if args.len() == 2 && std::path::Path::new(&args[1]).is_absolute() => {
            codepet_task_mcp::run_with_ledger(codepet_task_lineage::tasks::Ledger::new(
                std::path::Path::new(&args[1]),
            ))
        }
        None => codepet_task_mcp::run(),
        _ => Err("invalid task MCP arguments".into()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn install() -> Result<(), Box<dyn std::error::Error>> {
    let paths = codepet_paths::PathManager::discover()?;
    let config = codepet_task_mcp::managed::install(
        &std::env::current_exe()?,
        &paths.workspace,
        &paths.data,
    )?;
    println!("{config}");
    Ok(())
}
