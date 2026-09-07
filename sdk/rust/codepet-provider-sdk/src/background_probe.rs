//! Non-interactive probes use the shared platform process implementation.
use std::{io, process::Stdio, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Run a configured, non-interactive probe with bounded output and duration.
/// Cancellation drops the shared platform child guard and terminates its process tree.
pub async fn run(command: tokio::process::Command, timeout: Duration) -> io::Result<String> {
    let output = output(command, timeout).await?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "probe exited with {}",
            output.status
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Preserve exit status for machine interfaces that return structured data on nonzero exit.
pub async fn output(
    mut command: tokio::process::Command,
    timeout: Duration,
) -> io::Result<std::process::Output> {
    crate::local_runtime::runtime_environment(command.as_std_mut());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::spawn_async(command)?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // Dropping this future (stop, refresh, or shutdown) drops/kills the entire owned tree.
    let result = tokio::time::timeout(timeout, async {
        let (stdout, stderr, status) = tokio::try_join!(read(stdout), read(stderr), child.wait())?;
        Ok(std::process::Output {
            stdout,
            stderr,
            status,
        })
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "runtime probe timed out",
        )),
    }
}
async fn read(pipe: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    const LIMIT: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    pipe.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime probe output exceeded 1 MiB",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "subprocess fixture"]
    fn slow_child() {
        std::thread::sleep(Duration::from_secs(20));
    }
    #[test]
    #[ignore = "subprocess fixture"]
    fn large_child() {
        println!("{}", "x".repeat(2 * 1024 * 1024));
    }
    #[tokio::test]
    async fn command_timeout_and_output_limit_are_bounded() {
        let executable = std::env::current_exe().unwrap();
        let start = std::time::Instant::now();
        let mut command = tokio::process::Command::new(&executable);
        command.args([
            "--ignored",
            "--exact",
            "background_probe::tests::slow_child",
            "--nocapture",
        ]);
        let error = run(command, Duration::from_millis(100)).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(3));
        let mut command = tokio::process::Command::new(&executable);
        command.args([
            "--ignored",
            "--exact",
            "background_probe::tests::large_child",
            "--nocapture",
        ]);
        let error = run(command, Duration::from_secs(5)).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
