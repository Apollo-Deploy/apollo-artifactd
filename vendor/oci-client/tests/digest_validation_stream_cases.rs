enum StreamVariant {
    Stream,
    Partial,
}

// Regression test for https://github.com/oras-project/rust-oci-client/issues/64 (streaming path):
// pulling a blob that the registry doesn't have (404 with a BLOB_UNKNOWN envelope) must return a
// BlobUnknown registry error for both streaming variants.
#[rstest]
#[case::stream(StreamVariant::Stream)]
#[case::partial(StreamVariant::Partial)]
#[tokio::test]
async fn test_pull_blob_stream_not_found(#[case] variant: StreamVariant) {
    let server = BadServer::new(ServerConfig {
        bad_manifest: false,
        bad_config: false,
        bad_blob: false,
        blob_sha512: false,
        empty_digest: false,
    })
    .await;

    let client = Client::new(ClientConfig {
        protocol: ClientProtocol::Http,
        platform_resolver: Some(Box::new(linux_amd64_resolver)),
        ..Default::default()
    });

    let reference = Reference::try_from(format!(
        "{}/busybox@{}",
        server.server,
        MANIFEST_DIGEST.as_str()
    ))
    .expect("failed to parse reference");

    // A layer digest the mock registry doesn't know about, so the blob handler returns 404.
    let bad_layer = OciDescriptor {
        digest: "sha256:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .to_string(),
        ..Default::default()
    };

    let err = match variant {
        StreamVariant::Stream => client
            .pull_blob_stream(&reference, &bad_layer)
            .await
            .err()
            .expect("Expected an error streaming a missing blob"),
        StreamVariant::Partial => client
            .pull_blob_stream_partial(&reference, &bad_layer, 0, None)
            .await
            .err()
            .expect("Expected an error partially streaming a missing blob"),
    };
    assert!(
        matches!(&err, OciDistributionError::RegistryError { envelope, .. }
            if envelope.errors.iter().any(|e| e.code == OciErrorCode::BlobUnknown)),
        "Expected a BlobUnknown registry error, got: {err}"
    );
}
