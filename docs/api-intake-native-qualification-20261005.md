# Bounded API intake qualification

The daemon now uses rustix poll to retain at most 32 pending authorized sockets, with at most eight per kernel UID/GID principal and a five-second first-packet deadline. It selects a received packet before accepting another connection. Peer credentials and policy admission precede retaining a socket. No artifact executor thread or task is created per pending client. Artifact effects remain serialized; this change does not establish concurrent registry transfers or full multi-peer fairness.

Test-audit authoring gate: the public daemon test protects ready-request availability when earlier authorized connections send no packet. Serial accept/read is the credible regression; existing client cancellation/backlog tests do not exercise daemon packet admission. The test uses the real socket, daemon, and public client cancellation deadline without production seams.

From each native qualification checkout, run:

```text
cargo test --locked --test api_intake silent_authorized_connections_do_not_delay_ready_requests -- --exact
```

The x86 host `tihan-apollo` used `/usr/bin/cargo`; ARM `apollo-node-01` used `~/.cargo/bin/cargo`. Both passed 1/1 (0.02s and 0.03s respectively). The two silent client connections remained alive while the later STATUS request completed.

An isolated ARM source copy restored blocking listener/serial accept/read. The test failed after 2.11s with the intended error `idle authorized clients blocked ready status` and exit 101. The temporary source copy was removed automatically.

The control shared the qualification target cache, which caused the first subsequent current-source invocation to reuse the control executable despite unchanged current source hashes. That invocation failed in 2.03s. Touching the unchanged current API module forced recompilation from the authoritative checkout; the test then passed 1/1 in 0.15s. Future controls must use separate target directories. No production source was reverted or mutated by the negative control.

Source hashes verified against both native checkouts:

```text
src/api/mod.rs      25ac846cfeda6f0817193f7b840aadebe2e233210e4f86113330e851e22f9e00
src/api/intake.rs   02d08aed58b90abc9a9815e66ccba1d41ef6a9016b758bb943d0dab8fa18d9d2
tests/api_intake.rs bc1b94420fb1b3da515ae35f1c399c7dbd0de9cdca0213afcde2429f6db6cffb
```

Independent scoped review: [RED_TEAM_API_INTAKE.md](RED_TEAM_API_INTAKE.md). No full release approval is claimed.
