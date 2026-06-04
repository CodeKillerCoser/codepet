use crate::agent_control::{current_agent_statuses, set_all_agents_enabled, AgentStatus};

pub fn try_handle_cli() -> Result<bool, Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(false);
    }

    match args[0].as_str() {
        "--install-hooks" => {
            print_status(set_all_agents_enabled(true)?);
            Ok(true)
        }
        "--disable-hooks" => {
            print_status(set_all_agents_enabled(false)?);
            Ok(true)
        }
        "--status-hooks" => {
            print_status(current_agent_statuses()?);
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn print_status(statuses: Vec<AgentStatus>) {
    println!("{}", serde_json::to_string_pretty(&statuses).unwrap_or_default());
}
