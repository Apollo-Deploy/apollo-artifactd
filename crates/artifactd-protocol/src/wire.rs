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
    if let Some(rights) = rights.as_ref() {
        if !ancillary.push(SendAncillaryMessage::ScmRights(rights)) {
            return Err(std::io::Error::other("FD packet limit"));
        }
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
    let flags = if write {
        rustix::event::PollFlags::OUT
    } else {
        rustix::event::PollFlags::IN
    };
    let mut fds = [rustix::event::PollFd::new(&socket, flags)];
    let timeout = rustix::event::Timespec {
        tv_sec: 30,
        tv_nsec: 0,
    };
    let n = rustix::event::poll(&mut fds, Some(&timeout))?;
    if n != 1 || !fds[0].revents().contains(flags) {
        return Err(std::io::Error::other("socket timeout or hangup"));
    }
    Ok(())
}
