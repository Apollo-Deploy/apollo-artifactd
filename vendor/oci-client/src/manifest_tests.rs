    use super::*;

    const TEST_MANIFEST: &str = r#"{
        "schemaVersion": 2,
        "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
        "config": {
            "mediaType": "application/vnd.docker.container.image.v1+json",
            "size": 2,
            "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        },
        "artifactType": "application/vnd.wasm.component.v1+wasm",
        "layers": [
            {
                "mediaType": "application/vnd.wasm.content.layer.v1+wasm",
                "size": 1615998,
                "digest": "sha256:f9c91f4c280ab92aff9eb03b279c4774a80b84428741ab20855d32004b2b983f",
                "annotations": {
                    "org.opencontainers.image.title": "module.wasm"
                }
            }
        ]
    }
    "#;

    #[test]
    fn test_manifest() {
        let manifest: OciImageManifest =
            serde_json::from_str(TEST_MANIFEST).expect("parsed manifest");
        assert_eq!(2, manifest.schema_version);
        assert_eq!(
            Some(IMAGE_MANIFEST_MEDIA_TYPE.to_owned()),
            manifest.media_type
        );
        let config = manifest.config;
        // Note that this is the Docker config media type, not the OCI one.
        assert_eq!(IMAGE_DOCKER_CONFIG_MEDIA_TYPE.to_owned(), config.media_type);
        assert_eq!(2, config.size);
        assert_eq!(
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a".to_owned(),
            config.digest
        );
        assert_eq!(
            "application/vnd.wasm.component.v1+wasm".to_owned(),
            manifest.artifact_type.unwrap()
        );

        assert_eq!(1, manifest.layers.len());
        let wasm_layer = &manifest.layers[0];
        assert_eq!(1_615_998, wasm_layer.size);
        assert_eq!(WASM_LAYER_MEDIA_TYPE.to_owned(), wasm_layer.media_type);
        assert_eq!(
            1,
            wasm_layer
                .annotations
                .as_ref()
                .expect("annotations map")
                .len()
        );
    }
