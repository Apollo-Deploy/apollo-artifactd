use clap::Parser;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    store: std::path::PathBuf,
    #[arg(long)]
    socket: std::path::PathBuf,
}
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    #[cfg(target_os = "linux")]
    {
        apollo_artifactd::api::serve(&args.store, &args.socket)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("apollo-artifactd requires Linux SO_PEERCRED")
    }
}
