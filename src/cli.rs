use clap::Parser;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    socket: std::path::PathBuf,
    /// Trusted artifactd service UID; defaults to the caller UID.
    #[arg(long)]
    server_uid: Option<u32>,
    /// JSON-encoded generic protocol action; no credentials in command arguments.
    #[arg(long)]
    action: String,
    #[arg(long)]
    input: Option<std::path::PathBuf>,
    #[arg(long)]
    operation_id: Option<String>,
}
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    #[cfg(target_os = "linux")]
    {
        let client = artifactd_protocol::client::Client::new(
            &args.socket,
            args.server_uid
                .unwrap_or_else(|| rustix::process::geteuid().as_raw()),
        );
        let action: artifactd_protocol::Action = serde_json::from_str(&args.action)?;
        let operation_id = match args.operation_id {
            Some(operation_id) => operation_id.try_into().map_err(anyhow::Error::msg)?,
            None if action.is_mutation() => client.allocate()?,
            None => uuid::Uuid::new_v4()
                .to_string()
                .try_into()
                .map_err(anyhow::Error::msg)?,
        };
        let request = artifactd_protocol::Request {
            version: artifactd_protocol::VERSION,
            operation_id,
            action,
        };
        let input = args.input.map(std::fs::File::open).transpose()?;
        let (response, fd) = client.call(&request, input.as_ref())?;
        println!("{}", serde_json::to_string(&response)?);
        if let Some(fd) = fd {
            use std::os::fd::AsRawFd;
            eprintln!(
                "received descriptor {} (closed on CLI exit; use protocol client for FD ownership)",
                fd.as_raw_fd()
            );
        }
        anyhow::ensure!(response.result.is_ok(), "operation failed");
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("apollo-artifactctl requires Linux")
    }
}
