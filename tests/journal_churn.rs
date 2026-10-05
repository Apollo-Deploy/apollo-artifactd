#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use artifactd_protocol::{Action, OperationId, Request, VERSION, client};
use daemon::Daemon;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Instant};

const OPERATIONS: usize = 100_000;

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    rss_kib: u64,
    hwm_kib: u64,
    cpu_ticks: u64,
    threads: u64,
    fds: u64,
}

fn request(operation_id: OperationId, action: Action) -> Request {
    Request {
        version: VERSION,
        operation_id,
        action,
    }
}

fn mutation(socket: &Path, id: usize) -> Request {
    request(
        client::allocate(socket).unwrap(),
        Action::Unpin {
            id: format!("churn-{id}").try_into().unwrap(),
        },
    )
}

fn proc_value(pid: u32, name: &str) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key == name).then(|| value.trim().trim_end_matches(" kB").parse().unwrap())
        })
        .unwrap_or_default()
}

fn sample(daemon: &Daemon) -> Sample {
    let pid = daemon.0.id();
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let cpu_ticks = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    let fds = fs::read_dir(format!("/proc/{pid}/fd")).unwrap().count() as u64;
    Sample {
        rss_kib: proc_value(pid, "VmRSS"),
        hwm_kib: proc_value(pid, "VmHWM"),
        cpu_ticks,
        threads: proc_value(pid, "Threads"),
        fds,
    }
}

fn percentile(sorted: &[u128], numerator: usize, denominator: usize) -> u128 {
    sorted[((sorted.len() - 1) * numerator) / denominator]
}

fn root() -> (tempfile::TempDir, tempfile::TempDir) {
    let store = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for path in [store.path(), runtime.path()] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    (store, runtime)
}

#[test]
#[ignore = "100k native journal availability campaign"]
fn public_journal_churn_stays_bounded() {
    let (store, runtime) = root();
    let (daemon, socket) = Daemon::spawn(store.path(), runtime.path());
    let baseline = sample(&daemon);
    let mut latencies = Vec::with_capacity(OPERATIONS);
    let mut peak = baseline;
    let started = Instant::now();
    for id in 0..OPERATIONS {
        let request_started = Instant::now();
        let operation = mutation(&socket, id);
        assert!(
            client::call(&socket, &operation, None)
                .unwrap()
                .0
                .result
                .is_ok()
        );
        latencies.push(request_started.elapsed().as_micros());
        if id % 1000 == 999 {
            let current = sample(&daemon);
            peak.rss_kib = peak.rss_kib.max(current.rss_kib);
            peak.hwm_kib = peak.hwm_kib.max(current.hwm_kib);
            peak.cpu_ticks = peak.cpu_ticks.max(current.cpu_ticks);
            peak.threads = peak.threads.max(current.threads);
            peak.fds = peak.fds.max(current.fds);
        }
    }
    latencies.sort_unstable();
    assert!(
        peak.fds <= baseline.fds + 2,
        "fd growth: {baseline:?} -> {peak:?}"
    );
    assert!(
        peak.threads == baseline.threads,
        "thread growth: {baseline:?} -> {peak:?}"
    );
    assert!(
        peak.rss_kib <= baseline.rss_kib + 64 * 1024,
        "unbounded daemon RSS: {peak:?}"
    );
    eprintln!(
        "journal churn operations={OPERATIONS} elapsed_ms={} p50_us={} p95_us={} p99_us={} baseline={baseline:?} peak={peak:?}",
        started.elapsed().as_millis(),
        percentile(&latencies, 50, 100),
        percentile(&latencies, 95, 100),
        percentile(&latencies, 99, 100),
    );
    drop(daemon);

    let state_path = store.path().join("state.redb");
    let bytes = fs::metadata(state_path).unwrap().len();
    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = apollo_artifactd::state::State::open(&dir).unwrap();
    assert!(state.count("operations").unwrap() <= 4096);
    assert!(state.count("registry").unwrap() <= 4096);
    eprintln!(
        "journal churn state_bytes={bytes} operations={} registry={}",
        state.count("operations").unwrap(),
        state.count("registry").unwrap()
    );
    drop(state);

    let restart_runtime = tempfile::tempdir().unwrap();
    fs::set_permissions(restart_runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let (daemon, socket) = Daemon::spawn(store.path(), restart_runtime.path());
    let restart = mutation(&socket, OPERATIONS + 1);
    assert!(
        client::call(&socket, &restart, None)
            .unwrap()
            .0
            .result
            .is_ok()
    );
    drop(daemon);
}
