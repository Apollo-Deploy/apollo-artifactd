use super::*;

#[unsafe(no_mangle)]
pub extern "C" fn artifactd_operation_allocate(
    client: *mut ArtifactdClient,
    out: *mut c_char,
    capacity: usize,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let operation = client
            .inner
            .allocate()
            .map_err(|e| Error::Call(e.to_string()))?;
        output(operation.as_str(), out, capacity)
    })();
    result.map_or_else(fail, |_| succeed())
}

#[allow(clippy::too_many_arguments)]
fn pull_with_fd(
    client: &ArtifactdClient,
    operation: *const c_char,
    reference: *const c_char,
    os: *const c_char,
    arch: *const c_char,
    variant: *const c_char,
    pin_id: *const c_char,
    credentials_fd: i32,
) -> Result<serde_json::Value, Error> {
    let input = if credentials_fd < 0 {
        None
    } else {
        let owned = unsafe { File::from_raw_fd(credentials_fd) };
        let cloned = owned.try_clone().map_err(|e| Error::Call(e.to_string()))?;
        std::mem::forget(owned);
        Some(cloned)
    };
    let (value, _) = call_input(
        client,
        operation,
        Action::Pull {
            reference: text(reference)?.into(),
            platform: platform(os, arch, variant)?,
            pin: pin(pin_id)?,
        },
        input.as_ref(),
    )?;
    Ok(value)
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_pull(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    reference: *const c_char,
    os: *const c_char,
    arch: *const c_char,
    variant: *const c_char,
    pin_id: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        facts(
            pull_with_fd(client, operation, reference, os, arch, variant, pin_id, -1)?,
            out,
        )
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_pull_with_credentials(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    reference: *const c_char,
    os: *const c_char,
    arch: *const c_char,
    variant: *const c_char,
    pin_id: *const c_char,
    credentials_fd: i32,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        facts(
            pull_with_fd(
                client,
                operation,
                reference,
                os,
                arch,
                variant,
                pin_id,
                credentials_fd,
            )?,
            out,
        )
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_resolve(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    digest_value: *const c_char,
    os: *const c_char,
    arch: *const c_char,
    variant: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let (value, _) = call(
            client,
            operation,
            Action::Resolve {
                digest: digest(digest_value)?,
                platform: platform(os, arch, variant)?,
            },
        )?;
        facts(value, out)
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_verify(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    digest_value: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let (value, _) = call(
            client,
            operation,
            Action::Verify {
                digest: digest(digest_value)?,
            },
        )?;
        facts(value, out)
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_pin(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    pin_id: *const c_char,
    digest_value: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let (value, _) = call(
            client,
            operation,
            Action::Pin {
                id: id(pin_id, "pin")?
                    .try_into()
                    .map_err(|_| Error::Identity("pin".into()))?,
                digest: digest(digest_value)?,
            },
        )?;
        facts(value, out)
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_unpin(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    pin_id: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let (value, _) = call(
            client,
            operation,
            Action::Unpin {
                id: id(pin_id, "pin")?
                    .try_into()
                    .map_err(|_| Error::Identity("pin".into()))?,
            },
        )?;
        facts(value, out)
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_lease_create(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    digest_value: *const c_char,
    uid: u32,
    gid: u32,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let (value, _) = call(
            client,
            operation,
            Action::LeaseCreate {
                digest: digest(digest_value)?,
                grantee: Some(PeerIdentity { uid, gid }),
            },
        )?;
        facts(value, out)
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_lease_release(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    lease_id: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let (value, _) = call(
            client,
            operation,
            Action::LeaseRelease {
                id: id(lease_id, "lease")?
                    .try_into()
                    .map_err(|_| Error::Identity("lease".into()))?,
            },
        )?;
        facts(value, out)
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_prepare(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    digest_value: *const c_char,
    os: *const c_char,
    arch: *const c_char,
    variant: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let (value, _) = call(
            client,
            operation,
            Action::Prepare {
                digest: digest(digest_value)?,
                platform: platform(os, arch, variant)?,
            },
        )?;
        facts(value, out)
    })();
    result.map_or_else(fail, |_| succeed())
}
fn open(client: &ArtifactdClient, operation: *const c_char, action: Action) -> Result<i32, Error> {
    let (_, fd) = call(client, operation, action)?;
    fd.map(IntoRawFd::into_raw_fd)
        .ok_or_else(|| Error::Call("missing response FD".into()))
}

pub(crate) fn open_blob_fd(
    client: &ArtifactdClient,
    operation: *const c_char,
    digest_value: *const c_char,
    lease_id: *const c_char,
) -> Result<i32, Error> {
    open(
        client,
        operation,
        Action::OpenBlob {
            digest: digest(digest_value)?,
            lease: id(lease_id, "lease")?
                .try_into()
                .map_err(|_| Error::Identity("lease".into()))?,
        },
    )
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_open_blob(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    digest_value: *const c_char,
    lease_id: *const c_char,
    out_fd: *mut i32,
) -> i32 {
    let result = (|| {
        if out_fd.is_null() {
            return Err(Error::Null);
        }
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let fd = open(
            client,
            operation,
            Action::OpenBlob {
                digest: digest(digest_value)?,
                lease: id(lease_id, "lease")?
                    .try_into()
                    .map_err(|_| Error::Identity("lease".into()))?,
            },
        )?;
        unsafe { *out_fd = fd };
        Ok(())
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_open_prepared(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    prepared_id: *const c_char,
    lease_id: *const c_char,
    out_fd: *mut i32,
) -> i32 {
    let result = (|| {
        if out_fd.is_null() {
            return Err(Error::Null);
        }
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let fd = open(
            client,
            operation,
            Action::OpenPrepared {
                id: id(prepared_id, "prepared")?
                    .try_into()
                    .map_err(|_| Error::Identity("prepared".into()))?,
                lease: id(lease_id, "lease")?
                    .try_into()
                    .map_err(|_| Error::Identity("lease".into()))?,
            },
        )?;
        unsafe { *out_fd = fd };
        Ok(())
    })();
    result.map_or_else(fail, |_| succeed())
}
