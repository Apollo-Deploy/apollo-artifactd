# ARM64 native fuzz run interrupted — 2026-10-05

`apollo-node-01-internet` ran the native `protocol_oci` libFuzzer target from
the repaired 273-input production snapshot and 3,336-input fuzz-source
snapshot. The approved ARM64 fuzzer executable SHA-256 was
`e84700a02f55c4ee431d540133244f3bd175ba4073241ce51e12e87b4b9e783c`; its
initial corpus had 2,385 files and 127,718 bytes.

The run used `-max_total_time=1800 -rss_limit_mb=1024 -timeout=10
-max_len=65536 -print_final_stats=1 -verbosity=0`. Its mutable corpus lived
on `/dev/shm`; logs, progress and receipts were retained under
[`native-fuzz-arm64-aborted-20261005`](native-fuzz-arm64-aborted-20261005/).
After 480 seconds it had executed 20,109,482 units, added 32,637 corpus units,
reached a 557,232 KiB process HWM, used five FDs and produced no crash
artifacts.

The run was stopped before its 30-minute limit. During its last minute,
persistent filesystem free space fell from about 3.66 GiB to 3.20 GiB while
the campaign's persistent files remained small; the active corpus was on
tmpfs. To avoid adding storage pressure on the shared ARM host, the fuzzer was
stopped and its volatile corpus removed after recording its count and
SHA-256 manifest. The cause of the host's other disk growth was not
established. The captured `run.log` has SHA-256
`5649a55920aaf643b5249b601aa8cb6b6dc19b254ac73464fab538319187b362`.

A follow-up check over `ssh apollo-node-01-internet` confirmed that no process
from this campaign remained. The ARM root volume was still at 99% usage with
2,780,728 KiB free, so the campaign was not restarted and no unrelated host
files were removed.

This is a resource-limited, incomplete fuzz attempt, not a passing campaign.
No full native 30-minute fuzz target completed in this run. The seven fixed
recovery seeds and their deliberate negative control are separate passing
evidence; they do not substitute for the requested campaigns. Resume after
the host's persistent storage has stable headroom. Native x86 fuzzing and the
remaining property/fault matrix are also open.
