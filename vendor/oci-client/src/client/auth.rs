impl Client {
    /// Performs an OAuth v2 authentication request when required.
    pub async fn auth(
        &self,
        image: &Reference,
        authentication: &RegistryAuth,
        operation: RegistryOperation,
    ) -> Result<Option<String>> {
        self.store_auth_if_needed(image.resolve_registry(), authentication)
            .await;
        // preserve old caching behavior
        match self._auth(image, authentication, operation).await {
            Ok(Some(RegistryTokenType::Bearer(token))) => {
                self.tokens
                    .insert(image, operation, RegistryTokenType::Bearer(token.clone()))
                    .await;
                Ok(Some(token.token().to_string()))
            }
            Ok(Some(RegistryTokenType::Basic(username, password))) => {
                self.tokens
                    .insert(
                        image,
                        operation,
                        RegistryTokenType::Basic(username, password),
                    )
                    .await;
                Ok(None)
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Internal auth that retrieves token.
    async fn _auth(
        &self,
        image: &Reference,
        authentication: &RegistryAuth,
        operation: RegistryOperation,
    ) -> Result<Option<RegistryTokenType>> {
        debug!("Authorizing for image: {:?}", image);
        // The version request will tell us where to go.
        let url = format!(
            "{}://{}/v2/",
            self.config.protocol.scheme_for(image.resolve_registry()),
            image.resolve_registry()
        );

        if let RegistryAuth::Bearer(token) = authentication {
            return Ok(Some(RegistryTokenType::Bearer(RegistryToken::Token {
                token: token.clone(),
            })));
        }

        let res = self.client.get(&url).send().await?;
        let dist_hdr = match res.headers().get(reqwest::header::WWW_AUTHENTICATE) {
            Some(h) => h,
            None => return Ok(None),
        };

        let challenge = match BearerChallenge::try_from(dist_hdr) {
            Ok(c) => c,
            Err(e) => {
                debug!(error = ?e, "Falling back to HTTP Basic Auth");
                if let RegistryAuth::Basic(username, password) = authentication {
                    return Ok(Some(RegistryTokenType::Basic(
                        username.to_string(),
                        password.to_string(),
                    )));
                }
                return Ok(None);
            }
        };

        // Allow for either push or pull authentication
        let scope = match operation {
            RegistryOperation::Pull => format!("repository:{}:pull", image.repository()),
            RegistryOperation::Push => format!("repository:{}:pull,push", image.repository()),
        };

        let realm = Url::parse(challenge.realm.as_ref()).map_err(|_| {
            OciDistributionError::AuthenticationFailure(
                "authentication realm is not allowed".to_string(),
            )
        })?;
        if !is_allowed_auth_realm(
            &realm,
            image.resolve_registry(),
            self.config.protocol.scheme_for(image.resolve_registry()),
            &self.config.allowed_auth_realm_hosts,
        ) {
            return Err(OciDistributionError::AuthenticationFailure(
                "authentication realm is not allowed".to_string(),
            ));
        }
        let realm = realm.to_string();
        let service = challenge.service.as_ref();
        let mut query = vec![("scope", &scope)];

        if let Some(s) = service {
            query.push(("service", s))
        }

        debug!("Making authentication call");

        let auth_res = self
            .client
            .get(&realm)
            .query(&query)
            .apply_authentication(authentication)
            .send()
            .await
            .map_err(|_| {
                OciDistributionError::AuthenticationFailure(
                    "authentication request failed".to_string(),
                )
            })?;

        match auth_res.status() {
            reqwest::StatusCode::OK => {
                let body = read_bounded_response(auth_res, self.config.max_response_bytes).await?;
                debug!("Received response from auth request");
                let token: RegistryToken = serde_json::from_slice(&body)
                    .map_err(|e| OciDistributionError::RegistryTokenDecodeError(e.to_string()))?;
                debug!("Successfully authorized for image '{:?}'", image);
                Ok(Some(RegistryTokenType::Bearer(token)))
            }
            _ => {
                let _ = read_bounded_response(auth_res, self.config.max_response_bytes).await?;
                debug!("Authentication request failed");
                Err(OciDistributionError::AuthenticationFailure(
                    "authentication request failed".to_string(),
                ))
            }
        }
    }

}
