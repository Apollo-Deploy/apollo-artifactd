fn is_allowed_auth_realm(
    realm: &Url,
    registry_host: &str,
    registry_scheme: &str,
    allowed_hosts: &[String],
) -> bool {
    let host = realm.host_str().unwrap_or_default();
    let registry_url = Url::parse(&format!("{registry_scheme}://{registry_host}"));
    let same_registry = registry_url.ok().is_some_and(|registry| {
        registry.host_str() == Some(host)
            && registry.port_or_known_default() == realm.port_or_known_default()
    });
    (realm.scheme() == "https" || (realm.scheme() == "http" && registry_scheme == "http"))
        && realm.username().is_empty()
        && realm.password().is_none()
        && (same_registry
            || allowed_hosts
                .iter()
                .any(|allowed| allowed_authority_matches(realm, allowed)))
}

fn allowed_authority_matches(realm: &Url, allowed: &str) -> bool {
    let Ok(candidate) = Url::parse(&format!("{}://{}", realm.scheme(), allowed)) else {
        return false;
    };
    candidate.port().is_some()
        && candidate.host_str() == realm.host_str()
        && candidate.port_or_known_default() == realm.port_or_known_default()
}

fn redirect_allowed(previous: &[Url], next: &Url, max_redirects: usize) -> bool {
    if previous.len().saturating_sub(1) >= max_redirects
        || !next.username().is_empty()
        || next.password().is_some()
    {
        return false;
    }
    previous
        .first()
        .is_none_or(|initial| initial.scheme() != "https" || next.scheme() == "https")
}
