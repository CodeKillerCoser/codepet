pub fn try_handle_cli() -> Result<bool, Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("--install-hooks" | "--disable-hooks" | "--status-hooks") => {
            Err("旧 Hook 管理命令已停用，请在 App 的活动来源中启停 Provider 订阅".into())
        }
        _ => Ok(false),
    }
}
