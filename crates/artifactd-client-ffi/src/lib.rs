#![cfg(target_os = "linux")]
#![allow(clippy::not_unsafe_ptr_arg_deref)]
//! Thin Rust-owned C ABI over the authenticated artifactd protocol client.
//! Artifactd owns wire actions, OCI admission and verification, registry
//! behavior, storage, and extraction. The runtime-config helper uses the
//! mature `oci-spec` parser only after a verified leased config FD is opened;
//! it returns bounded runtime facts and does not implement artifact policy.
use artifactd_protocol::{
    Action, ArtifactDigest, OperationId, PeerIdentity, Platform, Request, Response, VERSION,
};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char},
    fs::File,
    os::fd::{FromRawFd, IntoRawFd, OwnedFd},
    path::Path,
};
use thiserror::Error;

#[derive(Debug, Error)]
enum Error {
    #[error("null pointer")]
    Null,
    #[error("invalid text argument")]
    Text,
    #[error("invalid identity: {0}")]
    Identity(String),
    #[error("invalid platform")]
    Platform,
    #[error("protocol call failed: {0}")]
    Call(String),
    #[error("output buffer too small")]
    Buffer,
}

thread_local! { static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) }; }
fn fail(error: impl std::fmt::Display) -> i32 {
    LAST_ERROR
        .with(|slot| *slot.borrow_mut() = CString::new(error.to_string().replace('\0', "")).ok());
    -1
}
fn succeed() -> i32 {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
    0
}
fn text<'a>(ptr: *const c_char) -> Result<&'a str, Error> {
    if ptr.is_null() {
        return Err(Error::Null);
    }
    // SAFETY: C callers provide NUL-terminated strings for this ABI.
    unsafe { CStr::from_ptr(ptr).to_str().map_err(|_| Error::Text) }
}
fn digest(ptr: *const c_char) -> Result<ArtifactDigest, Error> {
    text(ptr)?
        .parse::<ArtifactDigest>()
        .map_err(|e| Error::Identity(e.to_string()))
}
fn operation(ptr: *const c_char) -> Result<OperationId, Error> {
    text(ptr)?
        .to_owned()
        .try_into()
        .map_err(|_| Error::Identity("operation token".into()))
}
fn platform(
    os: *const c_char,
    arch: *const c_char,
    variant: *const c_char,
) -> Result<Platform, Error> {
    let value = Platform {
        os: text(os)?.into(),
        architecture: text(arch)?.into(),
        variant: if variant.is_null() {
            None
        } else {
            Some(text(variant)?.into())
        },
    };
    value.validate().then_some(value).ok_or(Error::Platform)
}
fn output(value: &str, dst: *mut c_char, capacity: usize) -> Result<(), Error> {
    if dst.is_null() {
        return Err(Error::Null);
    }
    if value.len() >= capacity {
        return Err(Error::Buffer);
    }
    // SAFETY: capacity was checked and caller owns the writable output buffer.
    unsafe {
        std::ptr::copy_nonoverlapping(value.as_ptr(), dst.cast::<u8>(), value.len());
        *dst.add(value.len()) = 0;
    }
    Ok(())
}
fn id(value: *const c_char, _kind: &str) -> Result<String, Error> {
    let value = text(value)?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Error::Identity("opaque id".into()));
    }
    Ok(value.to_owned())
}
fn response(response: Response) -> Result<serde_json::Value, Error> {
    response.result.map_err(Error::Call)
}

#[repr(C)]
pub struct ArtifactdClient {
    inner: artifactd_protocol::client::Client,
}
#[repr(C)]
pub struct ArtifactdFacts {
    value: serde_json::Value,
}

fn call_input(
    client: &ArtifactdClient,
    operation_ptr: *const c_char,
    action: Action,
    input: Option<&File>,
) -> Result<(serde_json::Value, Option<OwnedFd>), Error> {
    let request = Request {
        version: VERSION,
        operation_id: operation(operation_ptr)?,
        action,
    };
    let (wire_response, fd) = client
        .inner
        .call(&request, input)
        .map_err(|e| Error::Call(e.to_string()))?;
    Ok((response(wire_response)?, fd))
}
fn call(
    client: &ArtifactdClient,
    operation_ptr: *const c_char,
    action: Action,
) -> Result<(serde_json::Value, Option<OwnedFd>), Error> {
    call_input(client, operation_ptr, action, None)
}
fn facts(value: serde_json::Value, out: *mut *mut ArtifactdFacts) -> Result<(), Error> {
    if out.is_null() {
        return Err(Error::Null);
    }
    // SAFETY: ownership is transferred to the caller and released by facts_free.
    unsafe {
        *out = Box::into_raw(Box::new(ArtifactdFacts { value }));
    }
    Ok(())
}
fn pin(value: *const c_char) -> Result<Option<artifactd_protocol::PinId>, Error> {
    if value.is_null() {
        Ok(None)
    } else {
        Ok(Some(
            id(value, "pin")?
                .try_into()
                .map_err(|_| Error::Identity("pin".into()))?,
        ))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn artifactd_last_error(out: *mut c_char, capacity: usize) -> i32 {
    let value = LAST_ERROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    output(&value, out, capacity).map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_client_new(
    socket: *const c_char,
    uid: u32,
    out: *mut *mut ArtifactdClient,
) -> i32 {
    let result = (|| {
        if out.is_null() {
            return Err(Error::Null);
        }
        let path = text(socket)?;
        let value = Box::new(ArtifactdClient {
            inner: artifactd_protocol::client::Client::new(Path::new(path), uid),
        });
        unsafe {
            *out = Box::into_raw(value);
        }
        Ok(())
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_client_free(client: *mut ArtifactdClient) {
    if !client.is_null() {
        unsafe {
            drop(Box::from_raw(client));
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_free(value: *mut ArtifactdFacts) {
    if !value.is_null() {
        unsafe {
            drop(Box::from_raw(value));
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_fd_close(fd: i32) {
    if fd >= 0 {
        unsafe {
            drop(OwnedFd::from_raw_fd(fd));
        }
    }
}

mod actions;
mod facts;
mod runtime;
