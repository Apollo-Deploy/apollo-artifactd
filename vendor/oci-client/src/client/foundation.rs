impl Client {
    /// Create a new client with the supplied config
    pub fn new(config: ClientConfig) -> Self {
        let default_token_expiration_secs = config.default_token_expiration_secs;
        Client::try_from(config).unwrap_or_else(|err| {
            warn!("Cannot create OCI client from config: {:?}", err);
            warn!("Creating client with default configuration");
            Self {
                tokens: TokenCache::new(default_token_expiration_secs),
                push_chunk_size: PUSH_CHUNK_MAX_SIZE,
                ..Default::default()
            }
        })
    }

    /// Create a new client with the supplied config
    pub fn from_source(config_source: &impl ClientConfigSource) -> Self {
        Self::new(config_source.client_config())
    }

    async fn store_auth(&self, registry: &str, auth: RegistryAuth) {
        self.auth_store
            .write()
            .await
            .insert(registry.to_string(), auth);
    }

    async fn is_stored_auth(&self, registry: &str) -> bool {
        self.auth_store.read().await.contains_key(registry)
    }

    /// Store the authentication information for this registry if it's not already stored in the client.
    ///
    /// Most of the time, you don't need to call this method directly. It's called by other
    /// methods (where you have to provide the authentication information as parameter).
    ///
    /// But if you want to pull/push a blob without calling any of the other methods first, which would
    /// store the authentication information, you can call this method to store the authentication
    /// information manually.
    pub async fn store_auth_if_needed(&self, registry: &str, auth: &RegistryAuth) {
        if !self.is_stored_auth(registry).await {
            self.store_auth(registry, auth.clone()).await;
        }
    }

    /// Checks if we got a token, if we don't - create it and store it in cache.
    async fn get_auth_token(
        &self,
        reference: &Reference,
        op: RegistryOperation,
    ) -> Option<RegistryTokenType> {
        let registry = reference.resolve_registry();
        let auth = self.auth_store.read().await.get(registry)?.clone();
        match self.tokens.get(reference, op).await {
            Some(token) => Some(token),
            None => {
                let token = self._auth(reference, &auth, op).await.ok()??;
                self.tokens.insert(reference, op, token.clone()).await;
                Some(token)
            }
        }
    }

}
