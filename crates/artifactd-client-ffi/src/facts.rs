use super::*;

#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_json(
    value: *const ArtifactdFacts,
    out: *mut c_char,
    capacity: usize,
) -> i32 {
    let result = (|| {
        let value = unsafe { value.as_ref() }.ok_or(Error::Null)?;
        let text = serde_json::to_string(&value.value).map_err(|e| Error::Call(e.to_string()))?;
        output(&text, out, capacity)
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_get(
    value: *const ArtifactdFacts,
    key: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i32 {
    let result = (|| {
        let value = unsafe { value.as_ref() }.ok_or(Error::Null)?;
        let key = text(key)?;
        let field = value
            .value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::Call(format!("missing fact {key}")))?;
        output(field, out, capacity)
    })();
    result.map_or_else(fail, |_| succeed())
}
fn fact_text(
    value: *const ArtifactdFacts,
    key: &str,
    out: *mut c_char,
    capacity: usize,
) -> Result<(), Error> {
    let value = unsafe { value.as_ref() }.ok_or(Error::Null)?;
    let field = value
        .value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::Call(format!("missing fact {key}")))?;
    output(field, out, capacity)
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_artifact_digest(
    value: *const ArtifactdFacts,
    out: *mut c_char,
    capacity: usize,
) -> i32 {
    fact_text(value, "artifact_digest", out, capacity).map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_manifest_digest(
    value: *const ArtifactdFacts,
    out: *mut c_char,
    capacity: usize,
) -> i32 {
    fact_text(value, "manifest_digest", out, capacity).map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_config_digest(
    value: *const ArtifactdFacts,
    out: *mut c_char,
    capacity: usize,
) -> i32 {
    fact_text(value, "config_digest", out, capacity).map_or_else(fail, |_| succeed())
}
fn fact_u64(value: *const ArtifactdFacts, path: &[&str]) -> Result<u64, Error> {
    let value = unsafe { value.as_ref() }.ok_or(Error::Null)?;
    let mut current = &value.value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| Error::Call(format!("missing fact {key}")))?;
    }
    current
        .as_u64()
        .ok_or_else(|| Error::Call("fact is not an unsigned integer".into()))
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_manifest_size(
    value: *const ArtifactdFacts,
    out: *mut u64,
) -> i32 {
    let result = (|| {
        if out.is_null() {
            return Err(Error::Null);
        };
        let n = fact_u64(value, &["manifest_size"])?;
        unsafe { *out = n };
        Ok(())
    })();
    result.map_or_else(fail, |_| succeed())
}
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_facts_config_size(value: *const ArtifactdFacts, out: *mut u64) -> i32 {
    let result = (|| {
        if out.is_null() {
            return Err(Error::Null);
        };
        let n = fact_u64(value, &["config", "size"])?;
        unsafe { *out = n };
        Ok(())
    })();
    result.map_or_else(fail, |_| succeed())
}
