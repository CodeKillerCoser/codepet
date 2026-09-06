//! Run an executable in the same owned process boundary used by Providers.
fn main() -> std::io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let program = args
        .next()
        .ok_or_else(|| std::io::Error::other("expected executable and arguments"))?;
    let status = codepet_provider_sdk::process::Command::new(program)
        .args(args)
        .status()?;
    std::process::exit(status.code().unwrap_or(1));
}
