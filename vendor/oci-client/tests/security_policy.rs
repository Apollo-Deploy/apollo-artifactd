use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use axum::{
    http::{HeaderValue, StatusCode},
    routing::get,
    Router,
};
use oci_client::{
    client::{ClientConfig, ClientProtocol},
    secrets::RegistryAuth,
    Client, Reference, RegistryOperation,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

async fn raw_server(response: Vec<u8>, delay: Duration) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(delay).await;
            socket.write_all(&response).await.unwrap();
        }
    });
    (address, task)
}

fn client(config: ClientConfig) -> Client {
    Client::new(ClientConfig {
        protocol: ClientProtocol::Http,
        ..config
    })
}

#[tokio::test]
async fn public_manifest_pull_rejects_cumulative_chunked_metadata_limit() {
    let response = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Transfer-Encoding: chunked\r\n",
        "Content-Type: application/vnd.oci.image.manifest.v1+json\r\n",
        "Connection: close\r\n\r\n",
        "4\r\n1234\r\n",
        "4\r\n5678\r\n",
        "0\r\n\r\n"
    );
    let (address, task) = raw_server(response.as_bytes().to_vec(), Duration::ZERO).await;
    let client = client(ClientConfig {
        max_response_bytes: 6,
        ..Default::default()
    });
    let image = Reference::try_from(format!("{address}/repo:latest")).unwrap();

    let error = client
        .pull_manifest(&image, &RegistryAuth::Anonymous)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("byte limit"));
    task.await.unwrap();
}

#[tokio::test]
async fn public_manifest_pull_enforces_total_timeout() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec();
    let (address, task) = raw_server(response, Duration::from_millis(100)).await;
    let client = client(ClientConfig {
        total_timeout: Some(Duration::from_millis(20)),
        ..Default::default()
    });
    let image = Reference::try_from(format!("{address}/repo:latest")).unwrap();

    let error = client
        .pull_manifest(&image, &RegistryAuth::Anonymous)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("TimedOut"));
    task.await.unwrap();
}

#[tokio::test]
async fn public_auth_rejects_untrusted_bearer_realm_before_sending_basic_credentials() {
    let contacted = Arc::new(AtomicBool::new(false));
    let evil_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let evil_address = evil_listener.local_addr().unwrap().to_string();
    let evil_contacted = contacted.clone();
    let evil_task = tokio::spawn(async move {
        let (mut socket, _) = evil_listener.accept().await.unwrap();
        evil_contacted.store(true, Ordering::SeqCst);
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await;
    });

    let registry = Router::new().route(
        "/v2/",
        get(move || async move {
            (
                StatusCode::UNAUTHORIZED,
                [(
                    "WWW-Authenticate",
                    format!(
                    "Bearer realm=\"http://{evil_address}/token?user=secret\",service=\"registry\""
                ),
                )],
            )
        }),
    );
    let registry_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let registry_address = registry_listener.local_addr().unwrap().to_string();
    let registry_task = tokio::spawn(async move {
        axum::serve(registry_listener, registry).await.unwrap();
    });

    let client = client(ClientConfig::default());
    let image = Reference::try_from(format!("{registry_address}/repo:latest")).unwrap();
    let error = client
        .auth(
            &image,
            &RegistryAuth::Basic("user".into(), "password".into()),
            RegistryOperation::Pull,
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("realm is not allowed"));
    assert!(!contacted.load(Ordering::SeqCst));
    registry_task.abort();
    evil_task.abort();
}

#[tokio::test]
async fn public_manifest_pull_rejects_redirect_userinfo_without_contacting_target() {
    let target_contacted = Arc::new(AtomicBool::new(false));
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target_listener.local_addr().unwrap().to_string();
    let target_seen = target_contacted.clone();
    let target_task = tokio::spawn(async move {
        let (mut socket, _) = target_listener.accept().await.unwrap();
        target_seen.store(true, Ordering::SeqCst);
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await;
    });

    let redirect = format!(
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://user:password@{target_address}/repo:latest\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (address, redirect_task) = raw_server(redirect.into_bytes(), Duration::ZERO).await;
    let client = client(ClientConfig::default());
    let image = Reference::try_from(format!("{address}/repo:latest")).unwrap();

    let error = client
        .pull_manifest(&image, &RegistryAuth::Anonymous)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("redirect policy refused"));
    assert!(!target_contacted.load(Ordering::SeqCst));
    redirect_task.await.unwrap();
    target_task.abort();
}

#[tokio::test]
async fn public_manifest_push_rejects_upload_location_userinfo() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let location_address = address.clone();
    let router = Router::new().fallback(move || async move {
        (
            StatusCode::CREATED,
            [(
                "Location",
                format!("http://user:password@{location_address}/upload"),
            )],
        )
    });
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = client(ClientConfig::default());
    let image = Reference::try_from(format!("{address}/repo:latest")).unwrap();
    let content_type = HeaderValue::from_static("application/vnd.oci.image.manifest.v1+json");

    let error = client
        .push_manifest_raw(&image, br#"{}"#.to_vec(), content_type)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("must not contain userinfo"));
    task.abort();
}

#[tokio::test]
async fn public_manifest_push_handles_digest_reference_without_location() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let router = Router::new().fallback(|| async { StatusCode::CREATED });
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = client(ClientConfig::default());
    let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let image = Reference::try_from(format!("{address}/repo@{digest}")).unwrap();
    let content_type = HeaderValue::from_static("application/vnd.oci.image.manifest.v1+json");

    let location = client
        .push_manifest_raw(&image, br#"{}"#.to_vec(), content_type)
        .await
        .unwrap();
    assert!(location
        .ends_with("sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"));
    task.abort();
}
