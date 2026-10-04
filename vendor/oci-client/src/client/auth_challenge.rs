impl TryFrom<&ChallengeRef<'_>> for BearerChallenge {
    type Error = String;

    fn try_from(value: &ChallengeRef<'_>) -> std::result::Result<Self, Self::Error> {
        if !value.scheme.eq_ignore_ascii_case("Bearer") {
            return Err(format!(
                "BearerChallenge doesn't support challenge scheme {:?}",
                value.scheme
            ));
        }
        let mut realm = None;
        let mut service = None;
        for (key, value) in &value.params {
            if key.eq_ignore_ascii_case("realm") {
                realm = Some(value.to_unescaped());
            }
            if key.eq_ignore_ascii_case("service") {
                service = Some(value.to_unescaped());
            }
        }
        let realm = realm.ok_or("missing required parameter realm")?;
        Ok(Self {
            realm: realm.into_boxed_str(),
            service,
        })
    }
}
