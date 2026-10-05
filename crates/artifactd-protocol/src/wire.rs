//! Linux seqpacket transport. One bounded packet and at most one FD per request.
use crate::MAX_PACKET;
use rustix::{
    fd::{AsFd, OwnedFd},
    net::{
        self, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
        SendAncillaryBuffer, SendAncillaryMessage, SendFlags,
    },
};
use std::{
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
    time::{Duration, Instant},
};

pub fn receive(socket: impl AsFd) -> std::io::Result<(Vec<u8>, Option<OwnedFd>)> {
    let mut bytes = vec![0u8; MAX_PACKET];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let msg = net::recvmsg(
        socket,
        &mut [IoSliceMut::new(&mut bytes)],
        &mut ancillary,
        RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
    )?;
    let mut fd = None;
    let mut invalid = msg
        .flags
        .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        || msg.bytes == 0;
    for item in ancillary.drain() {
        match item {
            RecvAncillaryMessage::ScmRights(rights) => {
                for received in rights {
                    if fd.is_some() {
                        invalid = true;
                    } else {
                        fd = Some(received);
                    }
                }
            }
            _ => invalid = true,
        }
    }
    if invalid {
        return Err(std::io::Error::other("invalid or truncated packet"));
    }
    bytes.truncate(msg.bytes);
    Ok((bytes, fd))
}

pub fn send(socket: impl AsFd, bytes: &[u8], fd: Option<&OwnedFd>) -> std::io::Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_PACKET {
        return Err(std::io::Error::other("packet limit"));
    }
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    let rights = fd.map(|fd| [fd.as_fd()]);
    if let Some(rights) = rights.as_ref()
        && !ancillary.push(SendAncillaryMessage::ScmRights(rights))
    {
        return Err(std::io::Error::other("FD packet limit"));
    }
    let n = net::sendmsg(
        socket,
        &[IoSlice::new(bytes)],
        &mut ancillary,
        SendFlags::NOSIGNAL | SendFlags::DONTWAIT,
    )?;
    if n != bytes.len() {
        return Err(std::io::Error::other("short packet send"));
    }
    Ok(())
}

pub fn wait(socket: impl AsFd, write: bool) -> std::io::Result<()> {
    wait_for(socket, write, 30)
}

/// Bounded wait for an artifact operation response; packet delivery stays short.
pub fn wait_for(socket: impl AsFd, write: bool, seconds: u32) -> std::io::Result<()> {
    if !(1..=600).contains(&seconds) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid socket deadline",
        ));
    }
    wait_for_control(socket, write, seconds, || Ok(()))
}

/// Polls in one-second bounded intervals, allowing callers to cancel or enforce
/// an external deadline without blocking until the full operation timeout.
pub fn wait_for_control<F>(
    socket: impl AsFd,
    write: bool,
    seconds: u32,
    mut control: F,
) -> std::io::Result<()>
where
    F: FnMut() -> std::io::Result<()>,
{
    if !(1..=600).contains(&seconds) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid socket deadline",
        ));
    }
    let flags = if write {
        rustix::event::PollFlags::OUT
    } else {
        rustix::event::PollFlags::IN
    };
    let mut fds = [rustix::event::PollFd::new(&socket, flags)];
    let deadline = Instant::now() + Duration::from_secs(seconds.into());
    let terminal = rustix::event::PollFlags::ERR
        | rustix::event::PollFlags::HUP
        | rustix::event::PollFlags::NVAL;
    loop {
        control()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let timeout = rustix::event::Timespec {
            tv_sec: remaining.as_secs().min(1).try_into().unwrap_or(1),
            tv_nsec: if remaining.as_secs() == 0 {
                remaining.subsec_nanos().into()
            } else {
                0
            },
        };
        let n = rustix::event::poll(&mut fds, Some(&timeout))?;
        let revents = fds[0].revents();
        if n == 1 && revents.contains(flags) {
            return Ok(());
        }
        if revents.intersects(terminal) {
            return Err(std::io::Error::other("socket hangup or poll error"));
        }
    }
    Err(std::io::Error::other("socket timeout or hangup"))
}
