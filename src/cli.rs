use clap::Parser;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    socket: std::path::PathBuf,
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
        let request = artifactd_protocol::Request {
            version: artifactd_protocol::VERSION,
            operation_id: args
                .operation_id
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
                .try_into()
                .map_err(anyhow::Error::msg)?,
            action: serde_json::from_str(&args.action)?,
        };
        let input = args.input.map(std::fs::File::open).transpose()?;
        let (response, fd) = apollo_artifactd::api::call(&args.socket, &request, input.as_ref())?;
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
