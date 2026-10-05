use anyhow::{ensure, Context, Result};
use std::path::Path;

pub fn samples(values: &mut [u128]) -> serde_json::Value {
    values.sort_unstable();
    let at = |percent| {
        values
            .get(values.len().saturating_sub(1) * percent / 100)
            .copied()
            .unwrap_or(0)
    };
    serde_json::json!({"p50_us":at(50),"p95_us":at(95),"p99_us":at(99)})
}
pub fn resources(pid: u32, store: &Path) -> Result<serde_json::Value> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .with_context(|| format!("read daemon {pid} status"))?;
    let values: Vec<_> = status
        .lines()
        .filter(|line| {
            line.starts_with("VmRSS:") || line.starts_with("VmHWM:") || line.starts_with("Threads:")
        })
        .collect();
    ensure!(
        values.len() == 3,
        "daemon {pid} status missing resource fields"
    );
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .with_context(|| format!("read daemon {pid} fd directory"))?
        .count();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("read daemon {pid} stat"))?;
    let ticks: Vec<u64> = stat
        .split_whitespace()
        .skip(13)
        .take(2)
        .map(|value| value.parse().context("parse daemon CPU ticks"))
        .collect::<Result<_>>()?;
    ensure!(ticks.len() == 2, "daemon {pid} stat missing CPU fields");
    let state_bytes = std::fs::metadata(store.join("state.redb"))?.len();
    Ok(serde_json::json!({
        "pid":pid,
        "status":values,
        "fds":fds,
        "cpu_ticks":ticks[0] + ticks[1],
        "state_bytes":state_bytes
    }))
}
