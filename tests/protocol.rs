#![cfg(target_os = "linux")]
use artifactd_protocol::{MAX_PACKET, wire};
use rustix::{
    fd::AsFd,
    net::{
        self, AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags,
        SocketType,
    },
};
use std::{io::IoSlice, mem::MaybeUninit};

#[test]
fn truncated_packets_and_excess_descriptors_are_rejected() {
    let (a, b) = net::socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let bytes = vec![0u8; MAX_PACKET + 1];
    net::send(&a, &bytes, SendFlags::NOSIGNAL).unwrap();
    assert!(wire::receive(&b).is_err());
    let first = tempfile::tempfile().unwrap();
    let second = tempfile::tempfile().unwrap();
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    let rights = [first.as_fd(), second.as_fd()];
    assert!(ancillary.push(SendAncillaryMessage::ScmRights(&rights)));
    net::sendmsg(
        &a,
        &[IoSlice::new(b"{}")],
        &mut ancillary,
        SendFlags::NOSIGNAL,
    )
    .unwrap();
    assert!(wire::receive(&b).is_err());
    wire::send(&a, b"{}", None).unwrap();
    assert_eq!(wire::receive(&b).unwrap().0, b"{}");
}
