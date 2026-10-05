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
    path::{Path, PathBuf},
};

/// Authenticates the service UID before sending requests or descriptors.
/// Socket accessibility does not establish the identity of its listener.
pub struct Client {
    path: PathBuf,
    server_uid: u32,
}

impl Client {
    pub fn new(path: impl AsRef<Path>, server_uid: u32) -> Self {
        Self {
            path: path.as_ref().to_owned(),
            server_uid,
        }
    }

    pub fn call(
        &self,
        request: &Request,
        input: Option<&File>,
    ) -> Result<(Response, Option<OwnedFd>)> {
        self.call_with_control(request, input, || Ok(()))
    }

    /// Sends a request while periodically giving the caller a cancellation and
    /// deadline hook. If the hook fails after transmission, the operation token
    /// remains caller-owned because the server may still commit the operation.
    pub fn call_with_control<F>(
        &self,
        request: &Request,
        input: Option<&File>,
        mut control: F,
    ) -> Result<(Response, Option<OwnedFd>)>
    where
        F: FnMut() -> Result<()>,
    {
        control()?;
        let socket = net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )?;
        match net::connect(&socket, &SocketAddrUnix::new(&self.path)?) {
            Ok(()) => {}
            Err(error) if error == rustix::io::Errno::INPROGRESS => {
                wire::wait_for_control(&socket, true, 30, &mut control)?;
                match net::sockopt::socket_error(&socket)? {
                    Ok(()) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }
        if net::sockopt::socket_peercred(&socket)?.uid.as_raw() != self.server_uid {
            return Err(Error::other("unexpected artifactd owner"));
        }
        let fd = input
            .map(|f| f.try_clone().map(OwnedFd::from))
            .transpose()?;
        wire::wait_for_control(&socket, true, 30, &mut control)?;
        wire::send(
            &socket,
            &serde_json::to_vec(request).map_err(Error::other)?,
            fd.as_ref(),
        )?;
        // Streaming imports, registry transfers and rootfs verification can outlast
        // packet delivery. Preserve the token on timeout so callers can retry it.
        wire::wait_for_control(&socket, false, 600, &mut control)?;
        let (bytes, fd) = wire::receive(&socket)?;
        let response: Response = serde_json::from_slice(&bytes).map_err(Error::other)?;
        if response.version != VERSION || response.operation_id != request.operation_id {
            return Err(Error::other("response identity mismatch"));
        }
        Ok((response, fd))
    }

    pub fn allocate(&self) -> Result<OperationId> {
        self.allocate_with_control(|| Ok(()))
    }

    pub fn allocate_with_control<F>(&self, mut control: F) -> Result<OperationId>
    where
        F: FnMut() -> Result<()>,
    {
        let request = Request {
            version: VERSION,
            operation_id: "allocation".to_string().try_into().map_err(Error::other)?,
            action: Action::OperationAllocate,
        };
        let (response, _) = self.call_with_control(&request, None, &mut control)?;
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
}

/// Same-UID convenience client. Use `Client` for a dedicated service UID.
pub fn call(
    path: &Path,
    request: &Request,
    input: Option<&File>,
) -> Result<(Response, Option<OwnedFd>)> {
    Client::new(path, geteuid().as_raw()).call(request, input)
}

pub fn allocate(path: &Path) -> Result<OperationId> {
    Client::new(path, geteuid().as_raw()).allocate()
}
