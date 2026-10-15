use std::io::{Cursor, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MANIFEST: &[u8] = b"name: fixture\nversion: 1.0.0\n";

#[test]
fn extraction_errors_do_not_trigger_manifest_fallback() {
    for case in ["blocked", "zip", "yaml"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let install = home.join("plugins/fixture");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("manifest.yaml"), b"original manifest").unwrap();
        std::fs::write(install.join("marker"), b"preserve").unwrap();
        let body = if case == "yaml" {
            MANIFEST.to_vec()
        } else {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let entry = if case == "blocked" {
                "blocked/child.txt"
            } else {
                "manifest.yaml"
            };
            writer
                .start_file(
                    entry,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(MANIFEST).unwrap();
            let body = writer.finish().unwrap().into_inner();
            assert_eq!(zip::ZipArchive::new(Cursor::new(&body)).unwrap().len(), 1);
            body
        };
        if case == "blocked" {
            std::fs::write(install.join("blocked"), b"regular file").unwrap();
        }
        let payload = temp.path().join("payload");
        std::fs::write(&payload, &body).unwrap();
        let log = temp.path().join("log");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "marketplace_extract_worker",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("NO_PROXY", "*")
            .env("no_proxy", "*")
            .env("EXTRACT_CASE", case)
            .env("EXTRACT_PAYLOAD", payload)
            .env("EXTRACT_COMPLETE", temp.path().join("complete"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(outcome)) => break outcome,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{case}: child wait failed: {result:?}");
                }
            }
        };
        if case == "blocked" {
            assert_eq!(
                std::fs::read(install.join("manifest.yaml")).unwrap(),
                b"original manifest",
                "extraction failure overwrote original manifest"
            );
            assert_eq!(
                std::fs::read(install.join("blocked")).unwrap(),
                b"regular file"
            );
        } else {
            assert_eq!(
                std::fs::read(install.join("manifest.yaml")).unwrap(),
                MANIFEST
            );
            assert!(!install.join("plugin.zip").exists());
        }
        assert_eq!(std::fs::read(install.join("marker")).unwrap(), b"preserve");
        assert!(
            outcome.success(),
            "{case}: {}",
            std::fs::read_to_string(log).unwrap()
        );
        assert_eq!(
            std::fs::read(temp.path().join("complete")).unwrap(),
            b"checked"
        );
    }
}

#[test]
#[ignore = "bounded worker with owned home and loopback HTTP server"]
fn marketplace_extract_worker() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::config::config_home(), home);
    let case = std::env::var("EXTRACT_CASE").unwrap();
    let body = std::fs::read(std::env::var_os("EXTRACT_PAYLOAD").unwrap()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/fixture.zip", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&buffer[..read]);
                    assert!(request.len() < 16384);
                }
                assert!(request.starts_with(b"GET /fixture.zip HTTP/1.1\r\n"));
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            });
            let result = claurst_plugins::marketplace::marketplace_install(&url).await;
            server.await.unwrap();
            if case == "blocked" {
                let error = result.expect_err("valid ZIP extraction failure must not become YAML");
                assert!(error.starts_with("Extract archive:"), "{error}");
            } else {
                let installed = result.unwrap();
                assert_eq!(installed.name, "fixture");
                assert_eq!(installed.install_path, home.join("plugins/fixture"));
            }
        })
        .await
        .expect("loopback install exceeded deadline");
    });
    std::fs::write(std::env::var_os("EXTRACT_COMPLETE").unwrap(), b"checked").unwrap();
}
