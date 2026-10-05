//! Bounded packet admission. Idle authorized peers never occupy the executor.
use super::policy::Policy;
use crate::state::PeerIdentity;
use anyhow::Result;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fd::OwnedFd,
    net::{self, SocketFlags},
};
use std::time::{Duration, Instant};

const MAX_PENDING: usize = 32;
const MAX_PER_PEER: usize = 8;
const FIRST_PACKET_TIMEOUT: Duration = Duration::from_secs(5);

struct Pending {
    socket: OwnedFd,
    caller: PeerIdentity,
    deadline: Instant,
}

pub(super) struct Intake {
    pending: Vec<Pending>,
}

impl Intake {
    pub(super) fn new() -> Self {
        Self {
            pending: Vec::with_capacity(MAX_PENDING),
        }
    }

    pub(super) fn next(
        &mut self,
        listener: &OwnedFd,
        policy: &Policy,
    ) -> Result<(OwnedFd, PeerIdentity)> {
        loop {
            let now = Instant::now();
            self.pending.retain(|entry| entry.deadline > now);
            let accept = self.pending.len() < MAX_PENDING;
            let mut fds: Vec<_> = self
                .pending
                .iter()
                .map(|entry| PollFd::new(&entry.socket, PollFlags::IN))
                .collect();
            if accept {
                fds.push(PollFd::new(listener, PollFlags::IN));
            }
            let duration = self
                .pending
                .iter()
                .map(|entry| entry.deadline)
                .min()
                .map(|deadline| deadline.saturating_duration_since(now));
            let timeout = duration.map(|duration| Timespec {
                tv_sec: duration.as_secs().try_into().unwrap_or(5),
                tv_nsec: duration.subsec_nanos().into(),
            });
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            // Drain ready clients before admitting new connections, so a
            // connection flood cannot starve an already received request.
            let ready = fds
                .iter()
                .take(self.pending.len())
                .position(|fd| fd.revents().contains(PollFlags::IN));
            let closed: Vec<_> = fds
                .iter()
                .take(self.pending.len())
                .enumerate()
                .filter_map(|(index, fd)| {
                    fd.revents()
                        .intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL)
                        .then_some(index)
                })
                .collect();
            let listener_ready = accept
                && fds
                    .last()
                    .is_some_and(|fd| fd.revents().contains(PollFlags::IN));
            drop(fds);
            if let Some(index) = ready {
                let entry = self.pending.remove(index);
                return Ok((entry.socket, entry.caller));
            }
            for index in closed.into_iter().rev() {
                self.pending.remove(index);
            }
            if !listener_ready {
                continue;
            }
            let socket =
                match net::accept_with(listener, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK) {
                    Ok(socket) => socket,
                    Err(
                        rustix::io::Errno::AGAIN
                        | rustix::io::Errno::INTR
                        | rustix::io::Errno::CONNABORTED,
                    ) => continue,
                    Err(error) => return Err(error.into()),
                };
            let peer = net::sockopt::socket_peercred(&socket)?;
            let caller = PeerIdentity {
                uid: peer.uid.as_raw(),
                gid: peer.gid.as_raw(),
            };
            if policy.authorize_peer(&caller).is_err()
                || self
                    .pending
                    .iter()
                    .filter(|entry| entry.caller == caller)
                    .count()
                    >= MAX_PER_PEER
            {
                continue;
            }
            self.pending.push(Pending {
                socket,
                caller,
                deadline: Instant::now() + FIRST_PACKET_TIMEOUT,
            });
        }
    }
}
