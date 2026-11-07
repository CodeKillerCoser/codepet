use std::{io, path::Path, time::Duration};
pub(crate) async fn run(
    executable: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    data: Option<&Path>,
    timeout: Duration,
    client: Option<&crate::client::OpenCodeClient>,
) -> io::Result<String> {
    let mut command = tokio::process::Command::new(executable);
    codepet_provider_sdk::local_runtime::opencode_environment(command.as_std_mut(), data);
    command.args(args);
    if args != ["--version"] {
        if let Some(client) = client { client.configure_probe(command.as_std_mut()); }
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    codepet_provider_sdk::background_probe::run(command, timeout).await
}
