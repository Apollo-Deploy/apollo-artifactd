//! External consumers need only this protocol crate, never the daemon package.
use crate::{Action, OperationId, Request, Response, VERSION, wire};
use rustix::{
    fd::OwnedFd,
    net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType},
    process::geteuid,
};
use std::{
    fs::File,
    io::{Error, Result},
    path::Path,
};

pub fn call(
    path: &Path,
    request: &Request,
    input: Option<&File>,
) -> Result<(Response, Option<OwnedFd>)> {
    let socket = net::socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )?;
    net::connect(&socket, &SocketAddrUnix::new(path)?)?;
    if net::sockopt::socket_peercred(&socket)?.uid != geteuid() {
        return Err(Error::other("unexpected artifactd owner"));
    }
    let fd = input
        .map(|f| f.try_clone().map(OwnedFd::from))
        .transpose()?;
    wire::wait(&socket, true)?;
    wire::send(
        &socket,
        &serde_json::to_vec(request).map_err(Error::other)?,
        fd.as_ref(),
    )?;
    // Streaming imports, registry transfers and rootfs verification can outlast
    // packet delivery. Preserve the token on timeout so callers can retry it.
    wire::wait_for(&socket, false, 600)?;
    let (bytes, fd) = wire::receive(&socket)?;
    let response: Response = serde_json::from_slice(&bytes).map_err(Error::other)?;
    if response.version != VERSION || response.operation_id != request.operation_id {
        return Err(Error::other("response identity mismatch"));
    }
    Ok((response, fd))
}

pub fn allocate(path: &Path) -> Result<OperationId> {
    let request = Request {
        version: VERSION,
        operation_id: "allocation".to_string().try_into().map_err(Error::other)?,
        action: Action::OperationAllocate,
    };
    let (response, _) = call(path, &request, None)?;
    let value = response.result.map_err(Error::other)?;
    let token: OperationId = value
        .get("operation_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::other("operation allocation returned no operation token"))?
        .to_string()
        .try_into()
        .map_err(Error::other)?;
    token.token_parts().map_err(Error::other)?;
    Ok(token)
}
