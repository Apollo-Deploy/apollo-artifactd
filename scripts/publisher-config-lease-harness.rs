//! Public caller harness; compiled in the isolated publisher qualification workspace.
use apollo_buildkit_publisher::{
    ArtifactPublisher, PublishRequest, PublisherConfiguration, read_config_blob_with_control,
    reconcile_config_lease,
};
use artifactd_protocol::{Action, Request, VERSION, client::Client};
use std::{io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let socket = PathBuf::from(&args[1]);
    let uid = args[2].parse()?;
    let receipt_path = PathBuf::from(&args[4]);
    let intent_path = receipt_path.with_file_name("config-lease.json");
    if args[5] == "reconcile" {
        reconcile_config_lease(&intent_path, &socket, uid)?;
        println!("RECONCILED");
        return Ok(());
    }
    let publisher = ArtifactPublisher::new(PublisherConfiguration::new(socket.clone(), uid)?);
    let receipt = publisher.publish(&PublishRequest {
        oci_tar: args[3].clone().into(),
        receipt_path,
        build_attempt_id: "qualification-build".into(),
        source_snapshot_id: "qualification-source".into(),
        source_bundle_v1_digest: None,
    })?;
    let client = Client::new(&socket, uid);
    let result = read_config_blob_with_control(&socket, uid, &receipt, &intent_path, || {
        if args[5] == "normal" {
            return Ok(());
        }
        let (response, observed_fd) = client.call(
            &Request {
                version: VERSION,
                operation_id: "observation".to_string().try_into().unwrap(),
                action: Action::OpenBlob {
                    digest: receipt.config.digest.clone().try_into().unwrap(),
                    lease: match std::fs::read(
                        intent_path.with_file_name("config-lease.json.artifactd-operation.json"),
                    ) {
                        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)?["lease"]
                            .as_str()
                            .unwrap()
                            .to_owned()
                            .try_into()
                            .unwrap(),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                        Err(error) => return Err(error),
                    },
                },
            },
            None,
        )?;
        if response.result.is_ok() && observed_fd.is_some() {
            drop(observed_fd);
            if args[5] == "cancel" {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "qualification cancellation",
                ));
            }
            println!("LEASE_CREATED");
            std::io::stdout().flush()?;
            // The external owner sends SIGKILL after observing this public
            // daemon fact. No fixture directly writes the caller's intent.
            std::thread::sleep(std::time::Duration::from_secs(60));
            return Err(std::io::Error::other("qualification kill deadline"));
        }
        Ok(())
    });
    if args[5] == "cancel" {
        assert!(result.is_err(), "cancellation must interrupt config access");
        println!("CANCELLED");
    } else {
        let config: serde_json::Value = serde_json::from_slice(&result?)?;
        println!("{}", serde_json::json!({"receipt":receipt,"config":config}));
    }
    Ok(())
}
